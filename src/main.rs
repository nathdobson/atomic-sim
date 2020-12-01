#![feature(map_first_last)]
#![feature(bool_to_option)]
#![feature(cell_leak)]
#![feature(const_generics)]
#![feature(type_name_of_val)]
#![allow(unused_imports, unused_variables, incomplete_features, non_snake_case)]
#![deny(unused_must_use)]

use llvm_ir::Module;
use crate::thread::Thread;
use crate::process::Process;
use std::{fs, panic, mem};
use std::ffi::OsStr;
use rayon::prelude::*;
use crate::ctx::Ctx;
use std::panic::AssertUnwindSafe;
use crate::memory::{Memory};
use crate::symbols::Symbol;
use crate::function::Func;
use std::rc::Rc;
use std::borrow::Cow;

mod thread;
mod process;
mod layout;
mod value;
mod memory;
mod ctx;
mod function;
mod data;
mod symbols;
mod native;
mod interp;
mod backtrace;
mod flow;
mod compute;

pub fn main() {
    let mut modules = vec![];
    for file in fs::read_dir("tests/target-llvm-bc/x86_64-apple-darwin/release/deps/").expect("bad directory") {
        let path = file.expect("Could not read file").path();
        if path.extension() == Some(OsStr::new("bc"))
            && !path.file_stem().unwrap().to_str().unwrap().starts_with("panic_abort") {
            println!("Loading {:?}", path);
            modules.push(path);
        }
    }
    let modules = modules.par_iter().map(|path| Module::from_bc_path(path).expect("Could not parse module")).collect::<Vec<_>>();
    let modules = &modules;
    let native = native::builtins();
    let ctx = Rc::new(Ctx::new(modules, native));
    let mut process = Process::new(ctx.clone());
    process.add_thread(Symbol::External("main".to_string()));
    println!("{:?}", panic::catch_unwind(AssertUnwindSafe(|| {
        for i in 0.. {
            if !process.step() {
                break;
            }
            if i % 100000 == 0 {
                println!("{:?}", process.time());
            }
        }
    })));
    println!("{:#?}", process);
    mem::drop(process);
    mem::drop(ctx);
}