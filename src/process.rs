use crate::thread::{Thread};

use rand_xorshift::XorShiftRng;
use rand::SeedableRng;
use rand::Rng;
use std::rc::Rc;
use std::collections::{HashMap, BTreeMap};
use std::marker::PhantomData;
use std::fmt::{Debug, Formatter};
use std::{fmt, mem};
use llvm_ir::{Instruction, Type, Operand, Constant, IntPredicate, Name, Terminator, constant, TypeRef};
use llvm_ir::instruction::{Atomicity, InsertValue, SExt, ZExt, AtomicRMW, RMWBinOp, Select, Trunc, PtrToInt, ExtractValue, IntToPtr, CmpXchg};
use std::ops::{Range, Deref};
use crate::layout::{Layout, align_to, AggrLayout};
use llvm_ir::types::NamedStructDef;
use llvm_ir::constant::BitCast;
use std::borrow::Cow;
use crate::value::{Value, add_u64_i64};
use crate::memory::Memory;
use std::convert::TryInto;
use crate::ctx::{Ctx, EvalCtx, ThreadCtx};
use std::panic::UnwindSafe;
use crate::symbols::Symbol;


pub struct Process<'ctx> {
    pub ctx: &'ctx Ctx<'ctx>,
    pub threads: BTreeMap<usize, Thread<'ctx>>,
    pub next_threadid: usize,
    pub rng: XorShiftRng,
    pub memory: Memory<'ctx>,
}


impl<'ctx> Process<'ctx> {
    pub fn new(ctx: &'ctx Ctx, memory: Memory<'ctx>) -> Self {
        let mut process = Process {
            ctx,
            threads: BTreeMap::new(),
            next_threadid: 0,
            rng: XorShiftRng::seed_from_u64(0),
            memory,
        };
        process
    }
    pub fn ctx(&self) -> &'ctx Ctx<'ctx> {
        self.ctx
    }
    pub fn add_thread(&mut self, main: Symbol<'ctx>) {
        let threadid = self.next_threadid;
        self.next_threadid += 1;
        self.threads.insert(threadid,
                            Thread::new(self.ctx, main, threadid,
                                        &[Value::new(32, 0),
                                            Value::new(self.ctx.ptr_bits, 0)]));
    }
    pub fn step(&mut self) -> bool {
        if self.threads.is_empty() {
            return false;
        }
        let index = self.rng.gen_range(0, self.threads.len());
        let (&threadid, thread) = self.threads.iter_mut().nth(index).unwrap();
        if !thread.step()(self) {
            self.threads.remove(&threadid);
        }
        true
    }
    pub fn free(&mut self, tctx: ThreadCtx, ptr: &Value) {
        self.memory.free(ptr)
    }
    pub fn alloc(&mut self, tctx: ThreadCtx, layout: Layout) -> Value {
        self.memory.alloc(layout)
    }
    pub fn realloc(&mut self, tctx: ThreadCtx, old: &Value, old_layout: Layout, new_layout: Layout) -> Value {
        let new = self.alloc(tctx, new_layout);
        self.memcpy(tctx, &new, old, new_layout.bytes().min(old_layout.bytes()));
        self.free(tctx, old);

        new
    }
    pub fn store(&mut self, tctx: ThreadCtx, ptr: &Value, value: &Value, atomicity: Option<&'ctx Atomicity>) {
        self.memory.store(ptr, value, atomicity);
    }
    pub fn load(&mut self, tctx: ThreadCtx, ptr: &Value, layout: Layout, atomicity: Option<&'ctx Atomicity>) -> Value {
        self.memory.load(ptr, layout, atomicity)
    }
    pub fn memcpy(&mut self, tctx: ThreadCtx, dst: &Value, src: &Value, len: u64) {
        if len > 0 {
            let value = self.load(tctx, &src, Layout::from_bytes(len, 1), None);
            self.store(tctx, &dst, &value, None);
        }
    }
}

impl<'ctx> Debug for Process<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("threads", &self.threads)
            .finish()
    }
}
