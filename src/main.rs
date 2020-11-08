#![feature(map_first_last)]
#![allow(unused_imports)]

use llvm_ir::Module;
use flow::Ctx;
use crate::flow::Thread;
use crate::exec::Process;

mod flow;
mod exec;
mod layout;

pub fn main() {
    let module = Module::from_bc_path("tests/target/x86_64-apple-darwin/debug/deps/tests.bc").expect("module");
    let ctx = Ctx::new(&module);
    //let mut stack = Thread::new(&ctx, , 0);
    let mut process = Process::new(&ctx);
    process.add_thread("_ZN5tests4main17ha41f27c751ad6143E");
    while process.step() {}
}
