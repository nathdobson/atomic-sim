#![allow(unused_imports, unused_variables, incomplete_features, non_snake_case, dead_code, unused_macros)]

use std::thread;
use std::fmt::Debug;
use std::collections::{HashMap, BTreeSet, BTreeMap};
use std::hash::BuildHasher;


fn main() {
    // let mut map = HashMap::new();
    // map.insert(1, "a");
    // let mut map2 = BTreeMap::new();
    // map2.insert(2, "b");
    // println!("XXXX{:?}{:?}", map, map2);
    let mut map = HashMap::new();
    for i in 0..30000 {
        map.insert(i, i);
    }
    println!("{:?}", map.get(&42));
}