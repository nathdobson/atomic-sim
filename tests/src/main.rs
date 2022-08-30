#![allow(unused_imports, unused_variables, incomplete_features, non_snake_case, dead_code, unused_macros)]

use std::thread;
use std::fmt::Debug;
use std::collections::{HashMap, BTreeSet, BTreeMap};
use std::hash::BuildHasher;
use core::mem;
use std::sync::{Arc, Mutex, Barrier};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::cell::UnsafeCell;


fn fib(x: usize) -> usize {
    if x == 0 {
        0
    } else if x == 1 {
        1
    } else {
        fib(x - 1) + fib(x - 2)
    }
}

#[inline(never)]
fn factorial(x: usize) -> usize {
    if x == 0 {
        0
    } else {
        x * factorial(x - 1)
    }
}

fn par<T: 'static + Send>(c: usize, f: impl 'static + Send + Sync + Fn(usize) -> T) -> Vec<T> {
    let barrier = Arc::new(Barrier::new(c));
    let f = Arc::new(f);
    (0..c)
        .map(|v| thread::spawn({
            let barrier = barrier.clone();
            let f = f.clone();
            move || {
                barrier.wait();
                f(v)
            }
        }))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|x| x.join().unwrap())
        .collect::<Vec<_>>()
}

struct DangerCell<T>(UnsafeCell<T>);

impl<T> DangerCell<T> {
    unsafe fn new(x: T) -> Self {
        Self(UnsafeCell::new(x))
    }
    unsafe fn set(&self, x: T) {
        *self.0.get() = x;
    }
    unsafe fn get(&self) -> T where T: Copy {
        *self.0.get()
    }
}

unsafe impl<T> Send for DangerCell<T> {}

unsafe impl<T> Sync for DangerCell<T> {}

extern "C" {
    fn set_tracing(x: bool);
}
// extern "C" fn set_tracing(x:bool){}

fn main() {
    unsafe {
        let x = Arc::new(Mutex::new(0));
        for _ in 0..1 {
            println!("TEST");
            let a = DangerCell::new(1usize);
            let b = AtomicUsize::new(10usize);
            par(2, move |thread| {
                set_tracing(true);
                if thread == 0 {
                    a.set(2);
                    b.store(20, Ordering::Release);
                    set_tracing(false);
                } else if thread == 1 {
                    if b.load(Ordering::Acquire) == 20 {
                        let v = a.get();
                        set_tracing(false);
                        println!("Loaded {:?}", a.get());
                    } else {
                        set_tracing(false);
                        println!("Skipped");
                    }
                }
            });
        }
    }
}