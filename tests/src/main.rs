use std::thread;
use std::fmt::Debug;
use std::collections::{HashMap, BTreeSet};

#[inline(never)]
fn foo() -> u32 {
    let mut hashmap = HashMap::new();
    hashmap.insert(1, 1);
    hashmap.len() as u32
}

fn main() {
    //format!("{}{}", 1, 2);
    // let foo: HashMap<&str, &str> = [("a", "1"), ("b", "2")].iter().cloned().collect();
    // for x in foo {
    //     println!("XXXX{:?}", x);
    // }
    println!("XXX{:?}", foo());
    //thread::spawn(move || println!("Hello World")).join();
}