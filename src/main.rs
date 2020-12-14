#![feature(map_first_last)]
#![feature(bool_to_option)]
#![feature(cell_leak)]
#![feature(const_generics)]
#![feature(type_name_of_val)]
#![feature(or_patterns)]
#![feature(once_cell)]
#![feature(box_syntax)]
#![feature(duration_zero)]
#![feature(bound_cloned)]
#![feature(iter_map_while)]
#![feature(new_uninit)]
#![feature(maybe_uninit_extra)]
#![feature(maybe_uninit_ref)]
#![feature(thread_local)]
#![allow(unused_imports, unused_variables, incomplete_features, non_snake_case, dead_code, unused_macros)]
#![deny(unused_must_use, unconditional_recursion, private_in_public)]

#[global_allocator]
static GLOBAL: crate::allocator::Allocator = crate::allocator::Allocator;

use llvm_ir::Module;
use std::{fs, panic, mem};
use std::ffi::OsStr;
use std::rc::Rc;
use std::borrow::Cow;
use crate::compile::Compiler;
use crate::symbols::{SymbolTable, Symbol, ModuleId};
use crate::process::Process;
use std::panic::AssertUnwindSafe;
use std::thread::spawn;
use std::time::Instant;
use crate::timer::dump_trace;

//mod process;
mod layout;
mod value;
// mod memory;
// mod ctx;
// mod function;
// mod data;
mod symbols;
mod native;
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
mod thread;
mod flow;
mod function;
mod by_address;
mod lazy;
mod operation;
mod interp;
mod future;
mod rangemap;
mod freelist;
#[macro_use]
mod timer;
mod recursor;
mod allocator;

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
            modules.push(path);
        }
    }

    let modules =
        modules.into_iter()
            .map(|path|
                spawn(move ||
                    Module::from_bc_path(path).expect("Could not parse module")))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
    // for (mi, module) in modules.iter().enumerate() {
    //     println!("{:?}={:?}", ModuleId(mi), module.name);
    // }
    let (process, process_scope) = Process::new();
    process.add_native();
    let mut compiler = Compiler::new(process.clone());
    compiler.compile_modules(modules);

    process.add_thread(Symbol::External("main".to_string()));
    println!("{:?}", panic::catch_unwind(AssertUnwindSafe(|| {
        let start = Instant::now();
        for i in 0.. {
            if !process.step() {
                break;
            }
        }
        println!("{:?}", Instant::now() - start);
    })));
    mem::drop(process_scope);
    dump_trace();
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