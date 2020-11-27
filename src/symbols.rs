use std::collections::HashMap;
use crate::value::Value;
use crate::memory::{Memory};
use crate::layout::Layout;
use crate::ctx::EvalCtx;
use llvm_ir::module::Linkage;

pub struct SymbolTable<'ctx> {
    forward: HashMap<Symbol<'ctx>, Value>,
    reverse: HashMap<Value, Symbol<'ctx>>,
}

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Debug)]
pub enum Symbol<'ctx> {
    External(&'ctx str),
    Internal(usize, &'ctx str),
}

impl<'ctx> SymbolTable<'ctx> {
    pub fn new() -> Self {
        Self {
            forward: HashMap::new(),
            reverse: HashMap::new(),
        }
    }
    pub fn add_symbol(&mut self, memory: &mut Memory<'ctx>, name: Symbol<'ctx>, layout: Layout) -> Value {
        let address = memory.alloc(layout);
        assert!(self.forward.insert(name.clone(), address.clone()).is_none());
        assert!(self.reverse.insert(address.clone(), name).is_none());
        address
    }
    pub fn reverse_lookup(&self, address: &Value) -> Symbol<'ctx> {
        self.reverse.get(&address).unwrap_or_else(|| panic!("No symbol at {:?}", address)).clone()
    }
    pub fn lookup(&self, ectx: EvalCtx, name: &'ctx str) -> &Value {
        if let Some(internal) = self.forward.get(&Symbol::Internal(ectx.module.unwrap(), name)) {
            internal
        } else if let Some(external) = self.forward.get(&Symbol::External(name)) {
            external
        } else {
            panic!("No symbol named {:?}", name)
        }
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
