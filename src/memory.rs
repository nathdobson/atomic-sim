use std::collections::{BTreeMap, HashMap, HashSet};
use std::marker::PhantomData;
use crate::value::Value;
use llvm_ir::{Name, TypeRef};
use crate::layout::{Layout, align_to};
use llvm_ir::instruction::Atomicity;
use std::ops::Range;
use crate::ctx::{Ctx, EvalCtx, ThreadCtx};
use std::fmt::{Debug, Formatter};
use std::fmt;
use llvm_ir::module::Linkage;
use crate::exec::Process;

#[derive(Debug)]
pub struct Alloc<'ctx> {
    ptr: u64,
    len: u64,
    phantom: PhantomData<&'ctx ()>,
    stores: Vec<StoreLog>,
    freed: bool,
}

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Debug)]
pub enum Symbol<'ctx> {
    External(&'ctx str),
    Internal(usize, &'ctx str),
}

pub struct Memory<'ctx> {
    ctx: &'ctx Ctx<'ctx>,
    allocs: BTreeMap<u64, Alloc<'ctx>>,
    next: u64,
    symbols: HashMap<Symbol<'ctx>, Value>,
    reverse: HashMap<Value, Symbol<'ctx>>,
}

#[derive(Debug)]
struct StoreLog {
    pos: u64,
    len: u64,
    value: Vec<u8>,
}

const HEAP_GUARD: u64 = 128;

impl<'ctx> Memory<'ctx> {
    pub fn new(ctx: &'ctx Ctx<'ctx>) -> Self {
        Memory {
            ctx,
            allocs: BTreeMap::new(),
            next: 128,
            symbols: HashMap::new(),
            reverse: HashMap::new(),
        }
    }
    pub fn free(&mut self, ptr: &Value) {
        let freed =
            &mut self.allocs.get_mut(&ptr.as_u64())
                .unwrap_or_else(|| panic!("Cannot free {:?}", ptr.as_u64()))
                .freed;
        assert!(!*freed);
        *freed = true;
    }
    pub fn alloc(&mut self, layout: Layout) -> Value {
        let len = layout.bytes();
        let align = layout.byte_align();
        let result = align_to(self.next, align);
        self.next = result + len + HEAP_GUARD;
        self.allocs.insert(result, Alloc { ptr: result, len, phantom: PhantomData, stores: vec![], freed: false });
        Value::from(result)
    }
    fn find_alloc<'a>(&'a mut self, ptr: &Value) -> &'a mut Alloc<'ctx> where 'ctx: 'a {
        let alloc = self.allocs.range_mut(..=ptr.as_u64())
            .next_back()
            .unwrap_or_else(|| panic!("Could not find alloc {:?}", ptr)).1;
        assert!(!alloc.freed);
        alloc
    }
    pub fn store(&mut self, ptr: &Value, value: &Value, _atomicity: Option<&'ctx Atomicity>) {
        let value = value.bytes();
        let len = value.len() as u64;
        let alloc = self.find_alloc(ptr);
        let ptr = ptr.as_u64();
        assert!(alloc.ptr <= ptr && ptr + len <= alloc.ptr + alloc.len, "{:?} {:?} {:?}", alloc, ptr, len);
        alloc.stores.push(
            StoreLog {
                pos: ptr,
                len,
                value: value.to_vec(),
            }
        );
    }
    fn intersects(r1: Range<u64>, r2: Range<u64>) -> bool {
        r1.contains(&r2.start) || r2.contains(&r1.start)
    }
    pub fn load(&mut self, ptr: &Value, layout: Layout, _atomicity: Option<&'ctx Atomicity>) -> Value {
        let mut bytes = vec![0u8; layout.bytes() as usize];
        let mut missing = (0..layout.bytes()).collect::<HashSet<_>>();
        let alloc = self.find_alloc(ptr);
        for store in alloc.stores.iter().rev() {
            let ptr = ptr.as_u64();
            for i in missing.iter() {
                let r = store.pos..store.pos + store.len;
                let p = ptr + *i;
                if r.contains(&p) {
                    bytes[*i as usize] = store.value[(ptr + *i - store.pos) as usize];
                }
            }
            for i in store.pos..store.pos + store.len {
                if i >= ptr {
                    missing.remove(&(i - ptr));
                }
            }
            if missing.is_empty() {
                break;
            }
        }
        //assert!(missing.is_empty());
        Value::from_bytes(&bytes, layout)
    }
    pub fn add_symbol(&mut self, name: Symbol<'ctx>, layout: Layout) -> Value {
        let address = self.alloc(layout);
        //println!("Add symbol {:?} {:?} {:?}", name, layout, address);
        assert!(self.symbols.insert(name.clone(), address.clone()).is_none());
        assert!(self.reverse.insert(address.clone(), name).is_none());
        address
    }
    pub fn reverse_lookup(&self, address: &Value) -> Symbol<'ctx> {
        self.reverse.get(&address).unwrap_or_else(|| panic!("No symbol at {:?}", address)).clone()
    }
    pub fn lookup(&self, ectx: EvalCtx, name: &'ctx str) -> &Value {
        if let Some(internal) = self.symbols.get(&Symbol::Internal(ectx.module.unwrap(), name)) {
            internal
        } else if let Some(external) = self.symbols.get(&Symbol::External(name)) {
            external
        } else {
            panic!("No symbol named {:?}", name)
        }
    }
    pub fn ctx(&self) -> &'ctx Ctx<'ctx> {
        self.ctx
    }
}

impl<'ctx> Debug for Memory<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memory")
            .field("allocs", &self.allocs)
            .field("next", &self.next)
            .field("reverse", &self.reverse).finish()
    }
}

impl<'ctx> Symbol<'ctx> {
    pub fn new(linkage: Linkage, module: usize, name: &'ctx str) -> Self {
        match linkage {
            Linkage::Private | Linkage::Internal => Symbol::Internal(module, name),
            Linkage::External => Symbol::External(name),
            _ => todo!("{:?}", linkage)
        }
    }
}

impl<'ctx> Process<'ctx> {
    pub fn free(&mut self, tctx: ThreadCtx, ptr: &Value) {
        self.memory.free(ptr)
    }
    pub fn alloc(&mut self, tctx: ThreadCtx, layout: Layout) -> Value {
        self.memory.alloc(layout)
    }
    pub fn realloc(&mut self, tctx: ThreadCtx, old: &Value, old_layout: Layout, new_layout: Layout) -> Value {
        let new = self.alloc(tctx,new_layout);
        self.memcpy(tctx,&new, old, new_layout.bytes().min(old_layout.bytes()));
        self.free(tctx,old);

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
            let value = self.load(tctx,&src, Layout::from_bytes(len, 1), None);
            self.store(tctx,&dst, &value, None);
        }
    }
}