#![feature(core, core_slice_ext, box_patterns, cell_extras, unboxed_closures, fnbox)]
extern crate core;

use std::mem;
use std::rc::Rc;
use std::cell::RefCell;
use std::cell::Ref;
use std::boxed::FnBox;
use core::slice::SliceExt;

pub struct Promise<T> {
    state: Rc<RefCell<PromiseState<T>>>
}

enum PromiseState<T> {
    None,
    Value(T),
    Then(Box<FnBox(&T) -> ()>, Box<PromiseState<T>>),
    ThenMove(Box<FnBox(T) -> ()>)
}

impl<T> PromiseState<T> {
    fn is_value(&self) -> bool {
        if let &PromiseState::Value(_) = self {
            true
        } else {
            false
        }
    }
    fn transform(self, value: T) -> PromiseState<T> {
        match self {
            PromiseState::None => PromiseState::Value(value),
            PromiseState::Then(transform, box then) => {
                transform.call_box((&value,));
                then.transform(value)
            },
            PromiseState::ThenMove(transform) => {
                transform(value);
                PromiseState::None
            },
            _ => unreachable!()
        }
    }
}

trait ResolvableState<T> {
    fn resolve(&self, value: T);
}
impl<T> ResolvableState<T> for Rc<RefCell<PromiseState<T>>> {
    fn resolve(&self, value: T) {
        let mut s = self.borrow_mut();
        let state = mem::replace(&mut *s, PromiseState::None);
        *s = state.transform(value);
    }
}

impl<T: 'static> Promise<T> {
    pub fn new() -> Promise<T> {
        Promise {
            state: Rc::new(RefCell::new(PromiseState::None))
        }
    }
    pub fn resolved(value: T) -> Promise<T> {
        Promise {
            state: Rc::new(RefCell::new(PromiseState::Value(value)))
        }
    }
    pub fn resolve(&mut self, value: T) {
        self.state.resolve(value);
    }
    pub fn value(&self) -> Option<Ref<T>> {
        Ref::filter_map(self.state.borrow(), |state| match state {
            &PromiseState::Value(ref value) => Some(value),
            _ => None
        })
    }
    pub fn then_move<T2: 'static, F: FnOnce(T) -> T2 + 'static>(&mut self, transform: F) -> Promise<T2> {
        let p = Promise::<T2>::new();
        let p_state = p.state.clone();
        self._then_move(move |value| {
            let v2 = transform(value);
            p_state.resolve(v2);
        });
        p
    }
    pub fn then<T2: 'static, F: FnOnce(&T) -> T2 + 'static>(&mut self, transform: F) -> Promise<T2> {
        let p = Promise::<T2>::new();
        let p_state = p.state.clone();
        self._then(move |value| {
            let v2 = transform(value);
            p_state.resolve(v2);
        });
        p
    }
    pub fn then_move_promise<T2: 'static, F: FnOnce(T) -> Promise<T2> + 'static>(&mut self, transform: F) -> Promise<T2> {
        let p = Promise::<T2>::new();
        let p_state = p.state.clone();
        self._then_move(move |value| {
            let mut p2 = transform(value);
            let p_state = p_state.clone();
            p2._then_move(move |v2| {
                p_state.resolve(v2);
            });
        });
        p
    }
    pub fn then_promise<T2: 'static, F: FnOnce(&T) -> Promise<T2> + 'static>(&mut self, transform: F) -> Promise<T2> {
        let p = Promise::<T2>::new();
        let p_state = p.state.clone();
        self._then(move |value| {
            let mut p2 = transform(value);
            let p_state = p_state.clone();
            p2._then_move(move |v2| {
                p_state.resolve(v2);
            });
        });
        p
    }
    fn _then_move<F: FnOnce(T) -> () + 'static>(&mut self, transform: F) {
        if self.state.borrow().is_value() {
            let mut s = self.state.borrow_mut();
            if let PromiseState::Value(value) = mem::replace(&mut *s, PromiseState::None) {
                return transform(value);
            } else {
                unreachable!();
            }
        }
        *self.state.borrow_mut() = PromiseState::ThenMove(Box::new(move |value| {
            transform(value);
        }));
    }
    fn _then<F: FnOnce(&T) -> () + 'static>(&mut self, transform: F) {
        if let &PromiseState::Value(ref value) = &*self.state.borrow() {
            return transform(value);
        }
        let old_state = {
            let mut s = self.state.borrow_mut();
            mem::replace(&mut *s, PromiseState::None)
        };
        *self.state.borrow_mut() = PromiseState::Then(Box::new(move |value: &T| {
            transform(value);
        }), Box::new(old_state));
    }
}

pub fn join<T1: 'static, T2: 'static>(p1: &mut Promise<T1>, p2: &mut Promise<T2>) -> Promise<(T1, T2)> {
    (p1, p2).join()
}
pub fn join3<T1: 'static, T2: 'static, T3: 'static>(p1: &mut Promise<T1>, p2: &mut Promise<T2>, p3: &mut Promise<T3>) -> Promise<(T1, T2, T3)> {
    (p1, p2, p3).join()
}

pub trait Joinable<T> {
    fn join(self) -> Promise<T>;
}

impl<'a, T: 'static> Joinable<Vec<T>> for Vec<&'a mut Promise<T>> {
    fn join(mut self) -> Promise<Vec<T>> {
        let mut p: Promise<Vec<T>> = self[0].then_move(|x| vec![x]);
        for i in 1..self.len() {
            let mut p2 = &mut self[i];
            p = p2.then_move_promise(move |x| {
                p.then_move(move |mut xs: Vec<T>| { xs.push(x); xs })
            });
        }
        p
    }
}

impl<'a, T1: 'static, T2: 'static> Joinable<(T1, T2)> for (&'a mut Promise<T1>, &'a mut Promise<T2>) {
    fn join(mut self) -> Promise<(T1, T2)> {
        let mut p1 = Promise { state: self.1.state.clone() };
        self.0.then_move_promise(move |x1| {
            p1.then_move(move |x2| {
                (x1, x2)
            })
        })
    }
}

impl<'a, T1: 'static, T2: 'static, T3: 'static> Joinable<(T1, T2, T3)> for (&'a mut Promise<T1>, &'a mut Promise<T2>, &'a mut Promise<T3>) {
    fn join(mut self) -> Promise<(T1, T2, T3)> {
        let mut p1 = Promise { state: self.1.state.clone() };
        let mut p2 = Promise { state: self.2.state.clone() };
        self.0.then_move_promise(move |x1| {
            p1.then_move_promise(move |x2| {
                p2.then_move(move |x3| {
                    (x1, x2, x3)
                })
            })
        })
    }
}

#[test]
fn test_promise_resolve() {
    let mut p = Promise::new();
    p.resolve(5);
    assert_eq!(*p.value().unwrap(), 5);
}

#[test]
fn test_promise_resolved_then_promise() {
    let mut p = Promise::resolved(5);
    let p2 = p.then_promise(|val| Promise::resolved(val * 2));
    assert_eq!(*p2.value().unwrap(), 10);
}

#[test]
fn test_promise_then_promise() {
    let mut p = Promise::new();
    let p2 = p.then_promise(|val| Promise::resolved(val * 2));
    p.resolve(5);
    assert_eq!(*p2.value().unwrap(), 10);
}

#[test]
fn test_promise_resolved_then_move_promise() {
    let mut p = Promise::resolved(5);
    let p2 = p.then_move_promise(|val| Promise::resolved(val * 2));
    assert_eq!(*p2.value().unwrap(), 10);
}

#[test]
fn test_promise_then_move_promise() {
    let mut p = Promise::new();
    let p2 = p.then_move_promise(|val| Promise::resolved(val * 2));
    p.resolve(5);
    assert_eq!(*p2.value().unwrap(), 10);
}

#[test]
fn test_promise_then() {
    let mut p = Promise::new();
    let p2 = p.then(|val| val * 2);
    p.resolve(5);
    assert_eq!(*p2.value().unwrap(), 10);
}


#[test]
fn test_promise_join() {
    let mut a: Promise<i32> = Promise::new();
    let mut b: Promise<String> = Promise::new();
    let j = (&mut a, &mut b).join().then(|&(ref i, ref s)| format!("{} _ {}", i, s));
    assert!(j.value().is_none());
    a.resolve(5);
    assert!(j.value().is_none());
    b.resolve("hello".to_string());
    assert_eq!(*j.value().unwrap(), "5 _ hello".to_string());
}

#[test]
fn test_promise_join3() {
    let mut a: Promise<i32> = Promise::new();
    let mut b: Promise<String> = Promise::new();
    let mut c: Promise<String> = Promise::new();
    let j = (&mut a, &mut b, &mut c).join().then(|&(ref i, ref s, ref s2)| format!("{} _ {} {}", i, s, s2));
    assert!(j.value().is_none());
    a.resolve(5);
    assert!(j.value().is_none());
    b.resolve("hello".to_string());
    c.resolve("world".to_string());
    assert_eq!(*j.value().unwrap(), "5 _ hello world".to_string());
}

#[test]
fn test_promise_array_join() {
    let mut a: Promise<i32> = Promise::new();
    let mut b: Promise<i32> = Promise::new();
    let j: Promise<Vec<i32>> = vec![&mut a, &mut b].join();
    assert!(j.value().is_none());
    a.resolve(5);
    assert!(j.value().is_none());
    b.resolve(7);
    assert_eq!(*j.value().unwrap(), vec![5, 7]);
}
