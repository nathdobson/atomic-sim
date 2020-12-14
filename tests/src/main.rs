#![allow(unused_imports, unused_variables, incomplete_features, non_snake_case, dead_code, unused_macros)]

use std::thread;
use std::fmt::Debug;
use std::collections::{HashMap, BTreeSet, BTreeMap};
use std::hash::BuildHasher;
use core::mem;


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
    // let mut map = HashMap::new();
    // for i in 0..1000 {
    //     map.insert(i, i);
    // }
    //println!("{:?}", map.get(&42));
    println!("{:?}", factorial(50000));
}