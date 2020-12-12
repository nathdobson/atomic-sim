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
    for i in 0..1000 {
        map.insert(i, i);
    }
    println!("{:?}", map.get(&42));
}