use std::collections::HashMap;
use crate::value::Value;
use crate::memory::{Memory};
use crate::layout::Layout;
use crate::ctx::EvalCtx;
use llvm_ir::module::Linkage;
use std::fmt::{Debug, Formatter};
use std::fmt;
use std::borrow::Cow;
use std::marker::PhantomData;
use std::collections::hash_map::Entry;

pub struct SymbolTable<'ctx> {
    forward: HashMap<Symbol, Value>,
    reverse: HashMap<Value, Symbol>,
    phantom: PhantomData<&'ctx ()>,
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum Symbol {
    External(String),
    Internal(usize, String),
}

impl<'ctx> SymbolTable<'ctx> {
    pub fn new() -> Self {
        Self {
            forward: HashMap::new(),
            reverse: HashMap::new(),
            phantom: PhantomData,
        }
    }
    pub fn add_symbol(&mut self, memory: &mut Memory<'ctx>, name: Symbol, layout: Layout) -> Value {
        let address = memory.alloc(layout);
        assert!(self.forward.insert(name.clone(), address.clone()).is_none());
        assert!(self.reverse.insert(address.clone(), name).is_none());
        address
    }
    pub fn add_alias(&mut self, name: Symbol, address: Value) {
        match self.forward.entry(name.clone()) {
            Entry::Occupied(occupied) => { assert_eq!(occupied.get(), &address, "{:?}", name); }
            Entry::Vacant(vacant) => { vacant.insert(address); }
        }
    }
    pub fn reverse_lookup(&self, address: &Value) -> Symbol {
        self.reverse.get(&address).unwrap_or_else(|| panic!("No symbol at {:?}", address)).clone()
    }
    pub fn try_reverse_lookup(&self, address: &Value) -> Option<Symbol> {
        self.reverse.get(&address).cloned()
    }
    pub fn lookup(&self, ectx: EvalCtx, name: &str) -> &Value {
        if let Some(module) = ectx.module {
            if let Some(internal) = self.forward.get(&Symbol::Internal(module, name.to_string())) {
                return internal;
            }
        }
        if let Some(external) = self.forward.get(&Symbol::External(name.to_string())) {
            external
        } else {
            panic!("No symbol named {:?}", name)
        }
    }
    pub fn lookup_symbol(&self, sym: &Symbol) -> &Value {
        self.forward.get(sym).unwrap_or_else(|| panic!("No symbol {:?}", sym))
    }
}

impl Symbol {
    pub fn new(linkage: Linkage, module: usize, name: &str) -> Self {
        match linkage {
            Linkage::Private | Linkage::Internal => Symbol::Internal(module, name.to_string()),
            Linkage::External => Symbol::External(name.to_string()),
            _ => todo!("{:?}", linkage)
        }
    }
}

impl Debug for Symbol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Symbol::External(n) => write!(f, "{}", n),
            Symbol::Internal(i, n) => write!(f, "[{}]{}", i, n),
        }
    }
}