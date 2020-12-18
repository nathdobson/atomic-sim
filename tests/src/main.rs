#![allow(unused_imports, unused_variables, incomplete_features, non_snake_case, dead_code, unused_macros)]

use std::thread;
use std::fmt::Debug;
use std::collections::{HashMap, BTreeSet, BTreeMap};
use std::hash::BuildHasher;
use core::mem;
use std::sync::{Arc, Mutex, Barrier};


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

fn main() {
    let x = Arc::new(Mutex::new(0));
    for _ in 0..10 {
        println!("TEST");
        let barrier=Arc::new(Barrier::new(10));
        (0..10)
            .map(|v| {
                let barrier=barrier.clone();
                let x = x.clone();
                thread::spawn(move || {
                    barrier.wait();
                    println!("A{:?}", v);
                    *x.lock().unwrap() = v;
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .for_each(|t| t.join().unwrap());
        println!("{:?}", x);
    }
}