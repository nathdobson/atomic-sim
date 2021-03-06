use rand_xorshift::XorShiftRng;
use rand::SeedableRng;
use rand::Rng;
use std::rc::Rc;
use std::collections::{HashMap, BTreeMap, HashSet, VecDeque};
use std::marker::PhantomData;
use std::fmt::{Debug, Formatter};
use std::{fmt, mem};
use llvm_ir::{Instruction, Type, Operand, Constant, IntPredicate, Name, Terminator, constant, TypeRef};
use llvm_ir::instruction::{InsertValue, SExt, ZExt, AtomicRMW, RMWBinOp, Select, Trunc, PtrToInt, ExtractValue, IntToPtr, CmpXchg, MemoryOrdering};
use std::ops::{Range, Deref, DerefMut};
use crate::layout::{Layout, align_to, AggrLayout, Packing};
use llvm_ir::types::NamedStructDef;
use llvm_ir::constant::BitCast;
use std::borrow::{Cow};
use crate::value::{Value, add_u64_i64};
use std::convert::TryInto;
use std::panic::UnwindSafe;
use crate::symbols::{Symbol, SymbolTable};
use std::time::{Instant, Duration};
use crate::data::{Thunk};
use std::cell::RefCell;
use crate::function::Func;
use crate::memory::Memory;
use crate::thread::{ThreadId, Thread};
use crate::compile::class::{TypeMap, Class};
use crate::native;
use crate::timer;
use crate::symbols::SymbolDef;
use llvm_ir::module::ThreadLocalMode;
use crate::symbols::ThreadLocalKey;
use crate::compile::module::ModuleId;
use rand::seq::{SliceRandom, IteratorRandom};
use crate::scheduler::Scheduler;
use crate::ordering::Ordering;
use futures::executor::{LocalPool, LocalSpawner};
use futures::task::{SpawnExt, LocalSpawnExt};
use crate::compile::function::COperand::Local;
use crate::definition::Definitions;

pub struct ProcessInner {
    pub definitions: Definitions,
    pub ptr_bits: u64,
    pub page_size: u64,
    pub symbols: SymbolTable,
    pub memory: Memory,
    pub types: TypeMap,
    pub scheduler: Scheduler,
    pub pool: RefCell<LocalPool>,
    pub spawner: LocalSpawner,
}

#[derive(Clone)]
pub struct Process(Rc<ProcessInner>);

impl Deref for Process {
    type Target = ProcessInner;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

pub struct ProcessScope(Process);

impl Process {
    pub fn new() -> (Self, ProcessScope) {
        let ptr_bits = 64;
        let pool = LocalPool::new();
        let spawner = pool.spawner();
        let result = Process(Rc::new(ProcessInner {
            definitions: Definitions::new(),
            ptr_bits,
            page_size: 4096,
            symbols: SymbolTable::new(),
            memory: Memory::new(),
            types: TypeMap::new(ptr_bits),
            scheduler: Scheduler::new(),
            pool: RefCell::new(pool),
            spawner,
        }));
        (result.clone(), ProcessScope(result))
    }
    pub fn add_main(&self) {
        let fun = self.symbols.lookup_symbol(&Symbol::External("main".to_string())).global().unwrap();
        let params = &[Value::new(32, 0),
            Value::new(self.ptr_bits, 0)];
        self.add_thread(fun, params);
    }
    pub fn step(&self) -> bool {
        timer!("Process::step");
        let mut resolvable: Vec<Thunk> = vec![];
        loop {
            resolvable.clear();
            println!("Decoding... \n{:#?}", self.0.scheduler);
            self.pool.borrow_mut().run_until_stalled();
            let threads = self.0.scheduler.threads();
            let mut progress = false;
            println!("Stepping... \n{:#?}", self.0.scheduler);
            for thread in threads {
                progress |= thread.step_pure(&mut resolvable);
            }
            if !progress { break; }
        }
        println!("resolvable = {:?}", resolvable);
        if resolvable.len() == 0 {
            panic!("no progress to be made");
        } else if resolvable.len() == 1 {
            assert!(resolvable[0].step());
            true
        } else {
            panic!("No more available progress. {:#?}", self.0.scheduler);
        }
    }
    pub fn addr(&self, x: u64) -> Value {
        match self.ptr_bits {
            32 => assert!(x <= u32::MAX as u64),
            64 => assert!(x <= u64::MAX),
            _ => todo!(),
        }
        Value::from(x)
    }

    pub fn thread(&self, id: ThreadId) -> Option<Thread> {
        self.scheduler.thread(id)
    }

    pub fn add_thread(&self, fun: u64, params: &[Value]) -> ThreadId {
        let threadid = self.scheduler.new_threadid();
        let (thread, fut) = Thread::new(self.clone(), threadid, fun, params);
        self.spawner.spawn_local(fut).unwrap();
        self.scheduler.add_thread(thread);
        threadid
    }
    pub fn scheduler(&self) -> Scheduler {
        self.scheduler.clone()
    }
    pub fn add_native(&self) {
        for b in native::builtins() {
            let address =
                self.add_symbol(
                    Symbol::External(b.name().to_string()),
                    ThreadLocalMode::NotThreadLocal,
                    Layout::of_func());
            self.definitions.add_func(address.global().unwrap(), b);
        }
    }
    pub fn add_symbol(&self, name: Symbol, mode: ThreadLocalMode, layout: Layout) -> SymbolDef {
        let def = match mode {
            ThreadLocalMode::NotThreadLocal => {
                let address = self.memory.alloc(ThreadId::main(), layout);
                self.symbols.add_global(&name, address)
            }
            _ => self.symbols.add_thread_local(&name),
        };
        def
    }
}


impl Debug for Process {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("scheduler", &self.scheduler)
            .finish()
    }
}

impl Drop for ProcessScope {
    fn drop(&mut self) {
        self.0.scheduler.cancel();
    }
}
