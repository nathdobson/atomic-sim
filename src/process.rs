use crate::thread::{Thread, ThreadId};

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
use crate::ctx::{Ctx, EvalCtx};
use std::panic::UnwindSafe;
use crate::symbols::Symbol;
use std::time::{Instant, Duration};
use crate::data::Thunk;


pub struct Process<'ctx> {
    pub ctx: Rc<Ctx<'ctx>>,
    pub threads: BTreeMap<ThreadId, Thread<'ctx>>,
    pub next_threadid: usize,
    pub rng: XorShiftRng,
    pub memory: Memory<'ctx>,
    pub flow_time: Duration,
    pub compute_time: Duration,
}


impl<'ctx> Process<'ctx> {
    pub fn new(ctx: Rc<Ctx<'ctx>>) -> Self {
        Process {
            ctx: ctx.clone(),
            threads: BTreeMap::new(),
            next_threadid: 0,
            rng: XorShiftRng::seed_from_u64(0),
            memory: ctx.new_memory(),
            flow_time: Default::default(),
            compute_time: Default::default(),
        }
    }
    pub fn add_thread(&mut self, main: Symbol) {
        let threadid = ThreadId(self.next_threadid);
        self.next_threadid += 1;
        self.threads.insert(threadid,
                            Thread::new(self.ctx.clone(), main, threadid,
                                        &[Value::new(32, 0),
                                            Value::new(self.ctx.ptr_bits, 0)]));
    }
    pub fn step(&mut self) -> bool {
        if self.threads.is_empty() {
            return false;
        }
        let index = self.rng.gen_range(0, self.threads.len());
        let (&threadid, thread) = self.threads.iter_mut().nth(index).unwrap();
        let start = Instant::now();
        let step = thread.step();
        let middle = Instant::now();
        if !step(self) {
            self.threads.remove(&threadid);
        }
        let end = Instant::now();
        self.compute_time += end - middle;
        self.flow_time += middle - start;

        true
    }
    pub fn free(&mut self, tid: ThreadId, ptr: &Value) {
        self.memory.free(ptr)
    }
    pub fn alloc(&mut self, tid: ThreadId, layout: Layout) -> Value {
        self.memory.alloc(layout)
    }
    pub fn realloc(&mut self, tid: ThreadId, old: &Value, old_layout: Layout, new_layout: Layout) -> Value {
        let new = self.alloc(tid, new_layout);
        self.memcpy(tid, &new, old, new_layout.bytes().min(old_layout.bytes()));
        self.free(tid, old);

        new
    }
    pub fn store(&mut self, thread: ThreadId, ptr: &Value, value: &Value, atomicity: Option<&'ctx Atomicity>) {
        self.memory.store(ptr, value, atomicity);
    }
    pub fn load(&mut self, tid: ThreadId, ptr: &Value, layout: Layout, atomicity: Option<&'ctx Atomicity>) -> Value {
        self.memory.load(ptr, layout, atomicity)
    }
    pub fn memcpy(&mut self, tid: ThreadId, dst: &Value, src: &Value, len: u64) {
        if len > 0 {
            let value = self.load(tid, &src, Layout::from_bytes(len, 1), None);
            self.store(tid, &dst, &value, None);
        }
    }
    pub fn debug_info(&self, ptr: &Value) -> String {
        format!("{:?} {:?} {:?}", ptr, self.ctx.try_reverse_lookup(ptr), self.memory.debug_info(ptr))
    }
    pub fn time(&self) -> (Duration, Duration) {
        (self.flow_time, self.compute_time)
    }
}

impl<'ctx> Debug for Process<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("threads", &self.threads)
            .finish()
    }
}
