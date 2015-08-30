#![feature(core, unboxed_closures, core_slice_ext)]
extern crate core;

use std::rc::Rc;
use std::cell::Ref;
use std::ops::Deref;

use std::cell::RefCell;
use std::thread;
use std::sync::mpsc;
use std::sync::mpsc::*;
use std::mem;
use std::any::Any;
use core::slice::SliceExt;

pub struct Promise<T> {
    internal: Rc<RefCell<PromiseInternal<T>>>
}

impl<T> Clone for Promise<T> {
    fn clone(&self) -> Promise<T> {
        Promise { internal: self.internal.clone() }
    }
}

struct PromiseInternal<T> {
    value: Option<T>,
    then: Vec<Box<ApplyPromiseTransform<T>>>
}

struct PromiseTransform<T, T2> {
    promise: Rc<RefCell<PromiseInternal<T2>>>,
    transform: Box<Fn(&T) -> Promise<T2>>
}

trait ApplyPromiseTransform<T> {
    fn apply(&self, value: &T);
}

impl<T: 'static, T2: Clone + 'static> ApplyPromiseTransform<T> for PromiseTransform<T, T2> {
    fn apply(&self, value: &T) {
        let next_promise = self.transform.call((value, ));
        let had_value = {
            let pi = next_promise.internal.borrow();
            match &pi.value {
                &Some(ref value) => {
                    self.promise.borrow_mut().resolve(value.clone());
                    true
                },
                &None => false
            }
        };
        if !had_value {
            let p = self.promise.clone();
            next_promise.then(move |v| p.borrow_mut().resolve(v.clone()));
        }
    }
}

impl<T: 'static> PromiseInternal<T> {
    fn new() -> PromiseInternal<T> {
        PromiseInternal {
            value: None,
            then: vec![]
        }
    }
    fn resolved(value: T) -> PromiseInternal<T> {
        PromiseInternal {
            value: Some(value),
            then: vec![]
        }
    }
    fn resolve(&mut self, value: T) {
        for ref mut then in &self.then {
            then.apply(&value);
        }
        self.value = Some(value);
    }
}

impl<T: 'static> Promise<T> {
    pub fn new() -> Promise<T> {
        Promise {
            internal: Rc::new(RefCell::new(PromiseInternal::new())),
        }
    }
    pub fn resolved(value: T) -> Promise<T> {
        Promise {
            internal: Rc::new(RefCell::new(PromiseInternal::resolved(value))),
        }
    }
    pub fn resolve(&self, value: T) {
        let mut p = self.internal.borrow_mut();
        p.resolve(value);
    }
    pub fn value(&self) -> PromiseValue<T> {
        PromiseValue {
            internal_ref: self.internal.borrow()
        }
    }
    pub fn is_resolved(&self) -> bool {
        self.internal.borrow().value.is_some()
    }
    pub fn then<T2: Clone + 'static, F: Fn(&T) -> T2 + 'static>(&self, transform: F) -> Promise<T2> {
        self.then_promise(move |v| Promise::resolved(transform(v)))
    }
    pub fn then_promise<T2: Clone + 'static, F: Fn(&T) -> Promise<T2> + 'static>(&self, transform: F) -> Promise<T2> {
        let p = Rc::new(RefCell::new(PromiseInternal::new()));
        let mut int = self.internal.borrow_mut();
        int.then.push(Box::new(PromiseTransform {
            promise: p.clone(),
            transform: Box::new(transform)
        }));
        return Promise {
            internal: p.clone()
        }
    }
}

pub trait ToEmptyPromise {
    fn to_empty(&self) -> Promise<()>;
}

impl<T: 'static> ToEmptyPromise for Promise<T> {
    fn to_empty(&self) -> Promise<()> {
        self.then(move |_| {})
    }
}

pub fn join(promises: &[Box<ToEmptyPromise>]) -> Promise<()> {
    let mut p = promises.first().unwrap().clone().to_empty();
    for p2 in promises.tail() {
        let p2 = p2.clone().to_empty();
        p = p.then_promise(move |_| p2.clone());
    }
    p
}

pub struct PromiseValue<'a, T: 'a> {
    internal_ref: Ref<'a, PromiseInternal<T>>
}

impl<'a, T: 'a> Deref for PromiseValue<'a, T> {
    type Target = Option<T>;
    fn deref(&self) -> &Option<T> {
        &(*self.internal_ref).value
    }
}

pub trait Resolveable {
    fn try_resolve(&self) -> bool;
}

pub struct Running<T> {
    receiver: Receiver<T>,
    promise: Promise<T>
}

impl<T: 'static> Resolveable for Running<T> {
    fn try_resolve(&self) -> bool {
        match self.receiver.try_recv() {
            Ok(value) => {
                self.promise.resolve(value);
                true
            },
            _ => false
        }
    }
}

pub struct AsyncRunner {
    running: Vec<Box<Resolveable>>
}
impl AsyncRunner {
    pub fn new() -> AsyncRunner {
        AsyncRunner {
            running: vec![]
        }
    }
    pub fn exec_async<T: Send + Sized + 'static, F: Fn() -> T + Send + Sized + 'static>(&mut self, run: F) -> Promise<T> {
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            match tx.send(run()) {
                Ok(()) => {},
                Err(err) => panic!("Thread error: {}", err)
            }
        });
        let promise = Promise::new();
        self.running.push(Box::new(Running { receiver: rx, promise: promise.clone() }));
        promise
    }
    pub fn try_resolve_all(&mut self) {
        let running = mem::replace(&mut self.running, Vec::new());
        self.running = running.into_iter().filter(|r| !r.try_resolve()).collect();
    }
}

#[test]
fn test_promise_resolve() {
    let p = Promise::new();
    p.resolve(5);
    assert_eq!(p.value().clone(), Some(5));
}

#[test]
fn test_promise_then() {
    let p = Promise::new();
    let p2 = p.then(|val| val * 2);
    p.resolve(5);
    assert_eq!(p2.value().clone(), Some(10));
}

#[test]
fn test_promise_async() {
    let mut runner = AsyncRunner::new();
    let p = runner.exec_async(|| {
        thread::sleep_ms(10);
        "Hello world from thread".to_string()
    });
    runner.try_resolve_all();
    assert_eq!(p.value().clone(), None);
    thread::sleep_ms(20);
    runner.try_resolve_all();
    assert_eq!(p.value().clone(), Some("Hello world from thread".to_string()));
}

#[test]
fn test_promise_clone() {
    let p = Promise::new();
    let p2 = p.clone();
    p.resolve(5);
    assert_eq!(p2.value().clone(), Some(5));
}

#[test]
fn test_promise_join() {
    let a: Promise<i32> = Promise::new();
    let b: Promise<String> = Promise::new();
    let j = join(&[Box::new(a.clone()), Box::new(b.clone())]).then(|&()| 10);
    assert_eq!(*j.value(), None);
    a.resolve(5);
    assert_eq!(*j.value(), None);
    b.resolve("hello".to_string());
    assert_eq!(*j.value(), Some(10));
}
