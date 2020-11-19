use std::collections::{BTreeMap, HashMap, HashSet};
use std::marker::PhantomData;
use crate::value::Value;
use llvm_ir::{Name, TypeRef};
use crate::layout::Layout;
use llvm_ir::instruction::Atomicity;
use std::ops::Range;
use crate::ctx::{Ctx, EvalCtx};
use std::fmt::{Debug, Formatter};
use std::fmt;
use llvm_ir::module::Linkage;

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
        let len = layout.size();
        let align = layout.align();
        self.next = (self.next.wrapping_sub(1) | (align - 1)).wrapping_add(1);
        let result = self.next;
        self.next = self.next + len + HEAP_GUARD;
        self.allocs.insert(result, Alloc { ptr: result, len, phantom: PhantomData, stores: vec![], freed: false });
        Value::from(result)
    }
    pub fn realloc(&mut self, old: &Value, old_layout: Layout, new_layout: Layout) -> Value {
        let new = self.alloc(new_layout);
        assert_eq!(self.allocs[&old.as_u64()].len, old_layout.size());
        self.memcpy(&new, old, new_layout.size().min(old_layout.size()));
        self.free(old);

        new
    }
    fn find_alloc<'a>(&'a mut self, ptr: &Value) -> &'a mut Alloc<'ctx> where 'ctx: 'a {
        let alloc = self.allocs.range_mut(..=ptr.as_u64())
            .next_back()
            .unwrap_or_else(|| panic!("Could not find alloc {:?}", ptr)).1;
        assert!(!alloc.freed);
        alloc
    }
    pub fn store(&mut self, ptr: &Value, len: u64, value: &Value, _atomicity: Option<&'ctx Atomicity>) {
        let value = self.ctx.to_bytes(&value).1;
        assert_eq!(value.len() as u64, len);
        let alloc = self.find_alloc(ptr);
        let ptr = ptr.as_u64();
        assert!(alloc.ptr <= ptr && ptr + len <= alloc.ptr + alloc.len, "{:?} {:?} {:?}", alloc, ptr, len);
        alloc.stores.push(
            StoreLog {
                pos: ptr,
                len,
                value,
            }
        );
    }
    fn intersects(r1: Range<u64>, r2: Range<u64>) -> bool {
        r1.contains(&r2.start) || r2.contains(&r1.start)
    }
    pub fn load(&mut self, ptr: &Value, len: u64, _atomicity: Option<&'ctx Atomicity>, ty: &TypeRef) -> Value {
        let mut bytes = vec![0u8; len as usize];
        let mut missing = (0..len).collect::<HashSet<_>>();
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
        self.ctx.from_bytes(&bytes, ty)
    }
    pub fn add_symbol(&mut self, name: Symbol<'ctx>, layout: Layout) -> Value {
        let address = self.alloc(layout);
        assert!(self.symbols.insert(name.clone(), address.clone()).is_none());
        assert!(self.reverse.insert(address.clone(), name).is_none());
        address
    }
    pub fn reverse_lookup(&self, address: &Value) -> Symbol<'ctx> {
        self.reverse.get(&address).unwrap_or_else(|| panic!("No symbol at {:?}", address)).clone()
    }
    pub fn lookup(&self, ectx: &EvalCtx, name: &'ctx str) -> &Value {
        if let Some(internal) = self.symbols.get(&Symbol::Internal(ectx.module.unwrap(), name)) {
            internal
        } else if let Some(external) = self.symbols.get(&Symbol::External(name)) {
            external
        } else {
            panic!("No symbol named {:?}", name)
        }
    }
    pub fn memcpy(&mut self, dst: &Value, src: &Value, len: u64) {
        if len > 0 {
            let value = self.load(
                &src, len, None,
                &self.ctx.array_of(self.ctx.int(8), len as usize));
            self.store(&dst, len, &value, None);
        }
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