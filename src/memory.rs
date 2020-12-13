use std::collections::{BTreeMap, HashMap, HashSet};
use std::marker::PhantomData;
use crate::value::Value;
use llvm_ir::{Name, TypeRef};
use crate::layout::{Layout, align_to};
use llvm_ir::instruction::Atomicity;
use std::ops::Range;
use std::fmt::{Debug, Formatter};
use std::fmt;
use llvm_ir::module::Linkage;
use std::cell::RefCell;
use crate::thread::ThreadId;
use crate::by_address::ByAddress;
use std::rc::Rc;
use crate::rangemap::RangeMap;

#[derive(Debug)]
struct LogInner {
    start: u64,
    value: Vec<u8>,
    pred: RangeMap<u64, Log>,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
struct Log(ByAddress<Rc<LogInner>>);

#[derive(Clone)]
pub struct Memory {
    cells: RangeMap<u64, Log>,
    next: u64,
}

const HEAP_GUARD: u64 = 128;

impl Memory {
    pub fn new() -> Self {
        Memory {
            cells: RangeMap::new(),
            next: 128,
        }
    }
    pub fn free(&mut self, ptr: &Value) {
        //TODO
    }
    pub fn alloc(&mut self, layout: Layout) -> Value {
        let len = layout.bytes();
        let align = layout.byte_align();
        let result = align_to(self.next, align);
        self.next = result + len + HEAP_GUARD;
        Value::from(result)
    }
    pub fn store(&mut self, threadid: ThreadId, ptr: &Value, value: &Value, _atomicity: Option<Atomicity>) {
        let start = ptr.as_u64();
        let value = value.as_bytes().to_vec();
        let range = start..start + value.len() as u64;
        let pred =
            self.cells
                .range(range.clone())
                .map(|(k, v)| (k, v.clone()))
                .collect();
        let log = Log(ByAddress(Rc::new(LogInner { start, value, pred })));
        self.cells.insert(range, log);
    }
    pub fn load(&mut self, ptr: &Value, layout: Layout, _atomicity: Option<Atomicity>) -> Value {
        let start = ptr.as_u64();
        let range = start..start + layout.bytes();
        let mut result = Value::zero(layout.bits());
        for (r, v) in self.cells.range(range) {
            result.as_bytes_mut()[(r.start - start) as usize..(r.end - start) as usize]
                .copy_from_slice(&v.0.value[(r.start - v.0.start) as usize..(r.end - v.0.start) as usize]);
        }
        result
    }
    pub fn debug_info(&self, ptr: &Value) -> String {
        todo!()
    }
}

impl Debug for Memory {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memory")
            .field("cells", &self.cells)
            .finish()
    }
}
