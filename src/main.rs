#![feature(map_first_last)]
#![feature(bool_to_option)]
#![feature(cell_leak)]
#![allow(unused_imports)]

use llvm_ir::Module;
use crate::flow::Thread;
use crate::exec::Process;
use std::{fs, panic};
use std::ffi::OsStr;
use rayon::prelude::*;
use crate::ctx::Ctx;
use crate::native::NativeFunction;
use std::panic::AssertUnwindSafe;
use crate::memory::Symbol;


mod flow;
mod exec;
mod layout;
mod value;
mod memory;
mod ctx;
mod native;

pub fn main() {
    let mut modules = vec![];
    for file in fs::read_dir("tests/target-llvm-bc/x86_64-apple-darwin/debug/deps/").expect("bad directory") {
        let path = file.expect("Could not read file").path();
        if path.extension() == Some(OsStr::new("bc"))
            && !path.file_stem().unwrap().to_str().unwrap().starts_with("panic_abort") {
            println!("Loading {:?}", path);
            modules.push(path);
        }
    }
    let modules = modules.par_iter().map(|path| Module::from_bc_path(path).expect("Could not parse module")).collect::<Vec<_>>();
    let native = NativeFunction::builtins();
    println!("Running");
    let ctx = Ctx::new(&modules, &native);
    let mut process = Process::new(&ctx);
    process.add_thread(Symbol::External("main"));//_ZN5tests4main17ha41f27c751ad6143E
    println!("{:?}", panic::catch_unwind(AssertUnwindSafe(|| { while process.step() {} })));
    println!("{:#?}", process);
}
