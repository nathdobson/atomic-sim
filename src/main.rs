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
#![feature(generic_associated_types)]
#![feature(never_type)]
#![feature(btree_retain)]
#![allow(unused_imports, unused_variables, incomplete_features, non_snake_case, dead_code, unused_macros)]
#![deny(unused_must_use, unconditional_recursion, private_in_public)]

use std::{fs, mem, panic};
use std::borrow::Cow;
use std::ffi::OsStr;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;
use std::thread::spawn;
use std::time::Instant;

use llvm_ir::Module;

use crate::compile::module::Compiler;
use crate::process::Process;
use crate::symbols::{Symbol, SymbolTable};
use crate::util::timer::dump_trace;

#[global_allocator]
static GLOBAL: crate::util::allocator::Allocator = crate::util::allocator::Allocator;

mod layout;
mod value;
mod symbols;
mod native;
mod backtrace;
mod memory;
mod process;
mod data;
mod thread;
mod flow;
mod function;
mod interp;
#[macro_use]
mod compile;
mod util;
mod scheduler;
mod ordering;
mod definition;

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

    process.add_main();
    panic::catch_unwind(AssertUnwindSafe(|| {
        let start = Instant::now();
        for i in 0.. {
            if !process.step() {
                break;
            }
        }
        println!("Step time: {:?}", Instant::now() - start);
    })).ok();
    mem::drop(process_scope);
    dump_trace();
}