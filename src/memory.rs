use std::collections::{BTreeMap, HashMap, HashSet};
use std::marker::PhantomData;
use crate::value::Value;
use llvm_ir::{Name, TypeRef};
use crate::layout::{Layout, align_to};
use llvm_ir::instruction::{MemoryOrdering};
use std::ops::Range;
use std::fmt::{Debug, Formatter};
use std::fmt;
use llvm_ir::module::Linkage;
use std::cell::RefCell;
use crate::thread::ThreadId;
use std::rc::Rc;
use crate::util::rangemap::RangeMap;
use crate::util::by_address::ByAddress;
use crate::ordering::Ordering;

#[derive(Debug)]
struct LogInner {
    start: u64,
    value: Vec<u8>,
    pred: RangeMap<u64, Log>,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
struct Log(ByAddress<Rc<LogInner>>);

pub struct MemoryInner {
    cells: RangeMap<u64, Log>,
    next: u64,
}

pub struct Memory(RefCell<MemoryInner>);

const HEAP_GUARD: u64 = 128;

impl Memory {
    pub fn new() -> Self {
        Memory(RefCell::new(MemoryInner {
            cells: RangeMap::new(),
            next: 128,
        }))
    }
    pub fn free(&self, threadid: ThreadId, ptr: u64) {
        //TODO
    }
    pub fn alloc(&self, threadid: ThreadId, layout: Layout) -> u64 {
        let mut this = self.0.borrow_mut();
        let len = layout.bytes();
        let align = layout.byte_align();
        let result = align_to(this.next, align);
        this.next = result + len + HEAP_GUARD;
        result
    }
    pub fn store_impl(&self, threadid: ThreadId, ptr: u64, value: &Value, ordering: Ordering) {
        let mut this = self.0.borrow_mut();
        //println!("Store *{:?} = {:?}", ptr, value);
        let value = value.as_bytes().to_vec();
        let range = ptr..ptr + value.len() as u64;
        let pred =
            this.cells
                .range(range.clone())
                .map(|(k, v)| (k, v.clone()))
                .collect();
        let log = Log(ByAddress(Rc::new(LogInner { start: ptr, value, pred })));
        this.cells.insert(range, log);
    }
    pub fn load_impl(&self, threadid: ThreadId, ptr: u64, layout: Layout, ordering: Ordering) -> Value {
        let this = self.0.borrow();
        let range = ptr..ptr + layout.bytes();
        let mut result = Value::zero(layout.bits());
        for (r, v) in this.cells.range(range) {
            result.as_bytes_mut()[(r.start - ptr) as usize..(r.end - ptr) as usize]
                .copy_from_slice(&v.0.value[(r.start - v.0.start) as usize..(r.end - v.0.start) as usize]);
        }
        //println!("Load *{:?} = {:?}", ptr, result);
        result
    }
}

impl Debug for Memory {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let this = self.0.borrow();
        f.debug_struct("Memory")
            .field("cells", &this.cells)
            .finish()
    }
}
