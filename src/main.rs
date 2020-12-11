#![feature(map_first_last)]
#![feature(bool_to_option)]
#![feature(cell_leak)]
#![feature(const_generics)]
#![feature(type_name_of_val)]
#![feature(or_patterns)]
#![feature(once_cell)]
#![feature(box_syntax)]
#![feature(duration_zero)]
#![allow(unused_imports, unused_variables, incomplete_features, non_snake_case, dead_code)]
#![deny(unused_must_use, unconditional_recursion, private_in_public)]

use llvm_ir::Module;
use std::{fs, panic, mem};
use std::ffi::OsStr;
use rayon::prelude::*;
use std::rc::Rc;
use std::borrow::Cow;
use crate::compile::Compiler;
use crate::symbols::SymbolTable;
use crate::process::Process;

//mod process;
mod layout;
mod value;
// mod memory;
// mod ctx;
// mod function;
// mod data;
mod symbols;
// mod native;
// mod interp;
mod backtrace;
// mod flow;
// mod compute;
// mod arith;
mod class;
mod compile;
mod memory;
mod process;
mod data;
mod compute;
mod thread;
mod flow;
mod function;
mod by_address;
mod lazy;
mod arith;
mod operation;

pub fn main() {
    // use crate::thread::Thread;
    // use crate::process::Process;
    // use crate::ctx::Ctx;
    // use std::panic::AssertUnwindSafe;
    // use crate::memory::{Memory};
    // use crate::symbols::Symbol;
    //
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
    for (mi, module) in modules.iter().enumerate() {
        println!("{} {:?}", mi, module.name);
    }
    let (process, process_scope) = Process::new();
    let mut compiler = Compiler::new(64, process.clone());
    compiler.compile_modules(modules);
    mem::drop(process_scope);

    // let modules = &modules;
    // let native = native::builtins();
    // let ctx = Rc::new(Ctx::new(modules, native));
    // let mut process = Process::new(ctx.clone());
    // process.add_thread(Symbol::External("main".to_string()));
    // println!("{:?}", panic::catch_unwind(AssertUnwindSafe(|| {
    //     for i in 0.. {
    //         if !process.step() {
    //             break;
    //         }
    //         if i % 100000 == 0 {
    //             println!("{:?}", process.time());
    //         }
    //     }
    // })));
    // println!("{:#?}", process);
    // mem::drop(process);
    // mem::drop(ctx);
}