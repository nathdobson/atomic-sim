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

pub struct ProcessInner {
    functions: HashMap<u64, Rc<dyn Func>>,
    thread_local_inits: HashMap<ThreadLocalKey, (Layout, Value)>,
    ptr_bits: u64,
    page_size: u64,
    symbols: SymbolTable,
    memory: Memory,
    types: TypeMap,
    scheduler: Scheduler,
    pool: Rc<RefCell<LocalPool>>,
    spawner: LocalSpawner,
}

#[derive(Clone)]
pub struct Process(Rc<RefCell<ProcessInner>>);

pub struct ProcessScope(Process);

impl Process {
    pub fn new() -> (Self, ProcessScope) {
        let ptr_bits = 64;
        let pool=LocalPool::new();
        let spawner=pool.spawner();
        let result = Process(Rc::new(RefCell::new(ProcessInner {
            functions: HashMap::new(),
            thread_local_inits: HashMap::new(),
            ptr_bits,
            page_size: 4096,
            symbols: SymbolTable::new(),
            memory: Memory::new(),
            types: TypeMap::new(ptr_bits),
            scheduler: Scheduler::new(),
            pool: Rc::new(RefCell::new(pool)),
            spawner
        })));
        (result.clone(), ProcessScope(result))
    }
    pub fn types(&self) -> TypeMap {
        self.0.borrow().types.clone()
    }
    pub fn ptr_bits(&self) -> u64 {
        self.0.borrow().ptr_bits
    }
    pub fn add_main(&self) {
        let fun = self.lookup_symbol(&Symbol::External("main".to_string())).global().unwrap();
        let params = &[Value::new(32, 0),
            Value::new(self.ptr_bits(), 0)];
        self.add_thread(&self.value_from_address(fun), params);
    }
    pub fn step(&self) -> bool {
        timer!("Process::step");
        let pool = self.0.borrow().pool.clone();
        let mut resolvable;
        loop {
            resolvable = vec![];
            println!("Decoding... \n{:#?}", self.0.borrow().scheduler);
            pool.borrow_mut().run_until_stalled();
            let threads = self.0.borrow().scheduler.threads();
            let mut progress = false;
            println!("Stepping... \n{:#?}", self.0.borrow().scheduler);
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
            panic!("No more available progress. {:#?}", self.0.borrow().scheduler);
        }
    }
    pub fn free(&self, tid: ThreadId, ptr: &Value) {
        self.0.borrow_mut().memory.free(ptr)
    }
    pub fn alloc(&self, tid: ThreadId, layout: Layout) -> Value {
        self.0.borrow_mut().memory.alloc(layout)
    }
    pub fn store_impl(&self, threadid: ThreadId, ptr: &Value, value: &Value, atomicity: Ordering) {
        self.0.borrow_mut().memory.store_impl(threadid, ptr, value, atomicity);
    }
    pub fn load_impl(&self, threadid: ThreadId, ptr: &Value, layout: Layout, atomicity: Ordering) -> Value {
        self.0.borrow_mut().memory.load_impl(ptr, layout, atomicity)
    }
    pub fn debug_info(&self, ptr: &Value) -> String {
        let this = self.0.borrow_mut();
        format!("{:?} {:?} {:?}",
                ptr,
                this.symbols.try_reverse_lookup(ptr),
                this.memory.debug_info(ptr))
    }
    pub fn value_from_address(&self, x: u64) -> Value {
        let this = self.0.borrow_mut();
        match this.ptr_bits {
            32 => assert!(x <= u32::MAX as u64),
            64 => assert!(x <= u64::MAX),
            _ => todo!(),
        }
        Value::from(x)
    }
    pub fn null(&self) -> Value {
        self.value_from_address(0)
    }
    pub fn layout_of_ptr(&self) -> Layout {
        let this = self.0.borrow_mut();
        Layout::from_bits(this.ptr_bits, this.ptr_bits)
    }
    pub fn target_of(&self, ty: &TypeRef) -> TypeRef {
        match &**ty {
            Type::PointerType { pointee_type, .. } => pointee_type.clone(),
            _ => todo!("{:?}", ty),
        }
    }
    pub fn reverse_lookup(&self, address: &Value) -> Symbol {
        self.0.borrow().symbols.reverse_lookup(address)
    }
    pub fn reverse_lookup_fun(&self, address: &Value) -> Result<Rc<dyn Func>, String> {
        Ok(self.0.borrow().functions
            .get(&address.as_u64())
            .ok_or_else(|| format!("No such function {:?}", address))?
            .clone())
    }
    pub fn try_reverse_lookup(&self, address: &Value) -> Option<Symbol> {
        self.0.borrow().symbols.try_reverse_lookup(address)
    }
    pub fn lookup(&self, moduleid: Option<ModuleId>, name: &str) -> SymbolDef {
        self.0.borrow().symbols.lookup(moduleid, name)
    }
    pub fn lookup_symbol(&self, sym: &Symbol) -> SymbolDef {
        self.0.borrow().symbols.lookup_symbol(sym)
    }
    pub fn add_symbol(&self, symbol: Symbol, mode: ThreadLocalMode, layout: Layout) -> SymbolDef {
        let mut this = self.0.borrow_mut();
        let this = this.deref_mut();
        this.symbols.add_symbol(&mut this.memory, symbol, mode, layout)
    }
    pub fn add_alias(&self, symbol: Symbol, value: Value) {
        let mut this = self.0.borrow_mut();
        let this = this.deref_mut();
        this.symbols.add_alias(symbol, value);
    }
    pub fn add_func(&self, address: &Value, f: Rc<dyn Func>) {
        assert!(self.0.borrow_mut().functions.insert(address.as_u64(), f).is_none());
    }
    pub fn add_thread_local_init(&self, key: ThreadLocalKey, layout: Layout, value: Value) {
        assert!(self.0.borrow_mut().thread_local_inits.insert(key, (layout, value)).is_none());
    }
    pub fn thread_local_init(&self, key: ThreadLocalKey) -> (Layout, Value) {
        self.0.borrow().thread_local_inits.get(&key).unwrap().clone()
    }
    pub fn add_native(&self) {
        for b in native::builtins() {
            let address =
                self.add_symbol(
                    Symbol::External(b.name().to_string()),
                    ThreadLocalMode::NotThreadLocal,
                    Layout::of_func());
            self.0.borrow_mut().functions.insert(address.global().unwrap(), b);
        }
    }

    pub fn thread(&self, id: ThreadId) -> Option<Thread> {
        self.0.borrow().scheduler.thread(id)
    }

    pub fn add_thread(&self, fun: &Value, params: &[Value]) -> ThreadId {
        let threadid = self.0.borrow_mut().scheduler.new_threadid();
        let (thread, fut) = Thread::new(self.clone(), threadid, fun, params);
        self.0.borrow_mut().spawner.spawn_local(fut).unwrap();
        self.0.borrow_mut().scheduler.add_thread(thread);
        threadid
    }
    pub fn scheduler(&self) -> Scheduler {
        self.0.borrow().scheduler.clone()
    }
}


impl Debug for Process {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("scheduler", &self.0.borrow().scheduler)
            .finish()
    }
}

impl Drop for ProcessScope {
    fn drop(&mut self) {
        let this = self.0.0.borrow_mut();
        this.scheduler.cancel();
    }
}
