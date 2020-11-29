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
use crate::process::Process;

#[derive(Debug, Clone)]
pub struct Alloc<'ctx> {
    ptr: u64,
    len: u64,
    phantom: PhantomData<&'ctx ()>,
    stores: Vec<StoreLog>,
    freed: bool,
}

#[derive(Clone)]
pub struct Memory<'ctx> {
    allocs: BTreeMap<u64, Alloc<'ctx>>,
    next: u64,
}

#[derive(Clone)]
struct StoreLog {
    pos: u64,
    value: Vec<u8>,
}

const HEAP_GUARD: u64 = 128;

impl<'ctx> Memory<'ctx> {
    pub fn new() -> Self {
        Memory {
            allocs: BTreeMap::new(),
            next: 128,
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
    fn find_alloc_mut<'a>(&'a mut self, ptr: &Value) -> &'a mut Alloc<'ctx> where 'ctx: 'a {
        let alloc = self.allocs.range_mut(..=ptr.as_u64())
            .next_back()
            .unwrap_or_else(|| panic!("Could not find alloc {:?}", ptr)).1;
        assert!(!alloc.freed);
        alloc
    }
    fn find_alloc<'a>(&'a self, ptr: &Value) -> &'a Alloc<'ctx> where 'ctx: 'a {
        let alloc = self.allocs.range(..=ptr.as_u64())
            .next_back()
            .unwrap_or_else(|| panic!("Could not find alloc {:?}", ptr)).1;
        assert!(!alloc.freed);
        alloc
    }
    pub fn store(&mut self, ptr: &Value, value: &Value, _atomicity: Option<&'ctx Atomicity>) {
        let value = value.bytes();
        let len = value.len() as u64;
        let alloc = self.find_alloc_mut(ptr);
        let ptr = ptr.as_u64();
        assert!(alloc.ptr <= ptr && ptr + len <= alloc.ptr + alloc.len, "store overflow at {:?} length {:?} region {:?}", ptr, len, alloc);
        alloc.stores.push(
            StoreLog {
                pos: ptr,
                value: value.to_vec(),
            }
        );
    }
    pub fn load(&mut self, ptr: &Value, layout: Layout, _atomicity: Option<&'ctx Atomicity>) -> Value {
        let mut bytes = vec![0u8; layout.bytes() as usize];
        let mut missing = (0..layout.bytes()).collect::<HashSet<_>>();
        let alloc = self.find_alloc(ptr);
        for store in alloc.stores.iter().rev() {
            let ptr = ptr.as_u64();
            for i in missing.iter() {
                let r = store.pos..store.pos + store.value.len() as u64;
                let p = ptr + *i;
                if r.contains(&p) {
                    bytes[*i as usize] = store.value[(ptr + *i - store.pos) as usize];
                }
            }
            for i in store.pos..store.pos + store.value.len() as u64 {
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
    pub fn debug_info(&self, ptr: &Value) -> String {
        format!("{:?}", self.find_alloc(ptr))
    }
}

impl<'ctx> Debug for Memory<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memory")
            .field("allocs", &self.allocs)
            .finish()
    }
}

impl Debug for StoreLog {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreLog")
            .field("pos", &Value::from(self.pos))
            .field("value",
                   &Value::from_bytes(
                       &self.value,
                       Layout::from_bytes(self.value.len() as u64, 1)))
            .finish()
    }
}