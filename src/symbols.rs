use std::collections::HashMap;
use crate::value::Value;
use crate::layout::Layout;
use llvm_ir::module::Linkage;
use std::fmt::{Debug, Formatter};
use std::fmt;
use std::borrow::Cow;
use std::marker::PhantomData;
use std::collections::hash_map::Entry;
use std::cell::RefCell;
use crate::memory::Memory;

pub struct SymbolTable {
    forward: HashMap<Symbol, Value>,
    reverse: HashMap<Value, Symbol>,
}

#[derive(Copy, Clone, Eq, Ord, PartialEq, PartialOrd, Hash, Debug)]
pub struct ModuleId(pub usize);

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum Symbol {
    External(String),
    Internal(ModuleId, String),
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable{
            forward: HashMap::new(),
            reverse: HashMap::new(),
        }
    }
    pub fn add_symbol(&mut self, memory: &mut Memory, name: Symbol, layout: Layout) -> Value {
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
    pub fn lookup(&self, module: Option<ModuleId>, name: &str) -> Value {
        if let Some(module) = module {
            if let Some(internal) = self.forward.get(&Symbol::Internal(module, name.to_string())) {
                return internal.clone();
            }
        }
        if let Some(external) = self.forward.get(&Symbol::External(name.to_string())) {
            external.clone()
        } else {
            panic!("No symbol named {:?}", name)
        }
    }
    pub fn lookup_symbol(&self, sym: &Symbol) -> Value {
        self.forward.get(sym).unwrap_or_else(|| panic!("No symbol {:?}", sym)).clone()
    }
}

impl Symbol {
    pub fn new(linkage: Linkage, module: ModuleId, name: &str) -> Self {
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
            Symbol::Internal(i, n) => write!(f, "[{:?}]{}", i, n),
        }
    }
}