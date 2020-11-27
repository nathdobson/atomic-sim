#![feature(map_first_last)]
#![feature(bool_to_option)]
#![feature(cell_leak)]
#![allow(unused_imports)]
#![deny(unused_must_use)]

use llvm_ir::Module;
use crate::thread::Thread;
use crate::process::Process;
use std::{fs, panic};
use std::ffi::OsStr;
use rayon::prelude::*;
use crate::ctx::Ctx;
use std::panic::AssertUnwindSafe;
use crate::memory::{Memory};
use crate::symbols::Symbol;

mod thread;
mod process;
mod layout;
mod value;
mod memory;
mod ctx;
mod native;
mod data;
mod symbols;
mod frame;

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
    let native = native::builtins();
    println!("Running");
    let mut memory = Memory::new();
    let ctx = Ctx::new(&modules, &native, &mut memory);
    let mut process = Process::new(&ctx, memory);
    process.add_thread(Symbol::External("main"));
    println!("{:?}", panic::catch_unwind(AssertUnwindSafe(|| { while process.step() {} })));
    println!("{:#?}", process);
}
