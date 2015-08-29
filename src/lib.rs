#![feature(core, unboxed_closures)]

use std::rc::Rc;
use std::cell::Ref;
use std::ops::Deref;

use std::cell::RefCell;
use std::thread;
use std::sync::mpsc;
use std::sync::mpsc::*;

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
    transform: Box<Fn(&T) -> T2>
}

trait ApplyPromiseTransform<T> {
    fn apply(&self, value: &T);
}

impl<T: 'static, T2: 'static> ApplyPromiseTransform<T> for PromiseTransform<T, T2> {
    fn apply(&self, value: &T) {
        let next_val = self.transform.call((value, ));
        self.promise.borrow_mut().resolve(next_val);
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
    pub fn then<T2: 'static, F: Fn(&T) -> T2 + 'static>(&self, transform: F) -> Promise<T2> {
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

pub struct PromiseValue<'a, T: 'a> {
    internal_ref: Ref<'a, PromiseInternal<T>>
}

impl<'a, T: 'a> Deref for PromiseValue<'a, T> {
    type Target = Option<T>;
    fn deref(&self) -> &Option<T> {
        &(*self.internal_ref).value
    }
}

pub struct AsyncPromise<T> {
    pub promise: Promise<T>,
    receiver: Receiver<T>
}
impl<T: Send + 'static> AsyncPromise<T> {
    pub fn new<F: Fn() -> T + Send + Sized + 'static>(run: F) -> AsyncPromise<T> {
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            tx.send(run()).unwrap();
        });
        AsyncPromise {
            promise: Promise::new(),
            receiver: rx
        }
    }
    pub fn try_resolve(&self) -> bool {
        match self.receiver.try_recv() {
            Ok(value) => {
                self.promise.resolve(value);
                true
            },
            _ => false
        }
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
    let p = AsyncPromise::new(|| {
        thread::sleep_ms(10);
        "Hello world from thread".to_string()
    });
    p.try_resolve();
    assert_eq!(p.promise.value().clone(), None);
    thread::sleep_ms(20);
    p.try_resolve();
    assert_eq!(p.promise.value().clone(), Some("Hello world from thread".to_string()));
}

#[test]
fn test_promise_clone() {
    let p = Promise::new();
    let p2 = p.clone();
    p.resolve(5);
    assert_eq!(p2.value().clone(), Some(5));
}
