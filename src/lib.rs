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
use core::slice::SliceExt;

pub struct Promise<T> {
    internal: Rc<RefCell<PromiseInternal<T>>>
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
    fn then<T2: Clone + 'static, F: Fn(&T) -> T2 + 'static>(&self, transform: F) -> Promise<T2> {
        self.then_promise(move |v| Promise::resolved(transform(v)))
    }
    fn then_promise<T2: Clone + 'static, F: Fn(&T) -> Promise<T2> + 'static>(&self, transform: F) -> Promise<T2> {
        match &self.internal.borrow().value {
            &Some(ref value) => {
                return transform(value);
            },
            &None => {}
        }
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

pub trait Joinable<T> {
    fn join(self) -> Promise<T>;
}

impl<'a, T: 'static + Clone> Joinable<Vec<T>> for Vec<&'a Promise<T>> {
    fn join(self) -> Promise<Vec<T>> {
        let mut p: Promise<Vec<T>> = self[0].then(|x| vec![x.clone()]);
        for p2 in self.tail() {
            p = p2.then_promise(move |x| {
                let x = x.clone();
                p.then(move |xs: &Vec<T>| { let mut xs = xs.clone(); xs.push(x.clone()); xs })
            });
        }
        p
    }
}

pub fn join<'a, T1: 'static + Clone, T2: 'static + Clone>(p1: &'a Promise<T1>, p2: &'a Promise<T2>) -> Promise<(T1, T2)> {
    let p2 = Promise {
        internal: p2.internal.clone()
    };
    p1.then_promise(move |x1| {
        let x1 = x1.clone();
        p2.then(move |x2| {
            (x1.clone(), x2.clone())
        })
    })
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
    promise: Rc<RefCell<PromiseInternal<T>>>
}

impl<T: 'static> Resolveable for Running<T> {
    fn try_resolve(&self) -> bool {
        match self.receiver.try_recv() {
            Ok(value) => {
                self.promise.borrow_mut().resolve(value);
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
        self.running.push(Box::new(Running { receiver: rx, promise: promise.internal.clone() }));
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
fn test_promise_join() {
    let a: Promise<i32> = Promise::new();
    let b: Promise<String> = Promise::new();
    let j = join(&a, &b).then(|&(ref i, ref s)| format!("{} _ {}", i, s));
    assert_eq!(*j.value(), None);
    a.resolve(5);
    assert_eq!(*j.value(), None);
    b.resolve("hello".to_string());
    assert_eq!(*j.value(), Some("5 _ hello".to_string()));
}

#[test]
fn test_promise_array_join() {
    let a: Promise<i32> = Promise::new();
    let b: Promise<i32> = Promise::new();
    let j: Promise<Vec<i32>> = vec![&a, &b].join();
    assert_eq!(*j.value(), None);
    a.resolve(5);
    assert_eq!(*j.value(), None);
    b.resolve(7);
    assert_eq!(*j.value(), Some(vec![5, 7]));
}
#[test]
fn test_then_after_done() {
    let p = Promise::new();
    p.resolve(7);
    let p2 = p.then(|x| x * 2);
    assert_eq!(p2.value().clone(), Some(14));
}
