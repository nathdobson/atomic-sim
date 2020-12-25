use std::collections::HashMap;
use crate::value::Value;
use crate::layout::Layout;
use llvm_ir::module::{Linkage, ThreadLocalMode};
use std::fmt::{Debug, Formatter};
use std::fmt;
use std::borrow::Cow;
use std::marker::PhantomData;
use std::collections::hash_map::Entry;
use std::cell::RefCell;
use crate::memory::Memory;
use crate::compile::module::ModuleId;
use crate::thread::ThreadId;

#[derive(Copy, Clone, Debug, Ord, Eq, PartialEq, PartialOrd, Hash)]
pub struct ThreadLocalKey(usize);

pub struct SymbolTable(RefCell<SymbolTableInner>);

pub struct SymbolTableInner {
    forward: HashMap<Symbol, SymbolDef>,
    reverse: HashMap<u64, Symbol>,
    next_thread_local_key: usize,
}


#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum Symbol {
    External(String),
    Internal(ModuleId, String),
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Debug)]
pub enum SymbolDef {
    Global(u64),
    ThreadLocal(ThreadLocalKey),
}

impl SymbolDef {
    pub fn global(&self) -> Option<u64> {
        match self {
            SymbolDef::Global(x) => Some(*x),
            _ => None,
        }
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable(RefCell::new(SymbolTableInner {
            forward: HashMap::new(),
            reverse: HashMap::new(),
            next_thread_local_key: 0,
        }))
    }

    pub fn add_alias(&self, name: Symbol, address: u64) {
        let mut this = self.0.borrow_mut();
        let def = SymbolDef::Global(address);
        match this.forward.entry(name.clone()) {
            Entry::Occupied(occupied) => { assert_eq!(occupied.get(), &def, "{:?}", name); }
            Entry::Vacant(vacant) => { vacant.insert(def); }
        }
    }
    pub fn reverse_lookup(&self, address: u64) -> Symbol {
        let this = self.0.borrow();
        this.reverse.get(&address).unwrap_or_else(|| panic!("No symbol at {:?}", address)).clone()
    }
    pub fn try_reverse_lookup(&self, address: u64) -> Option<Symbol> {
        let this = self.0.borrow();
        this.reverse.get(&address).cloned()
    }
    pub fn lookup(&self, module: Option<ModuleId>, name: &str) -> SymbolDef {
        let this = self.0.borrow();
        if let Some(module) = module {
            if let Some(internal) = this.forward.get(&Symbol::Internal(module, name.to_string())) {
                return internal.clone();
            }
        }
        if let Some(external) = this.forward.get(&Symbol::External(name.to_string())) {
            external.clone()
        } else {
            println!("No symbol named {:?}", name);
            this.forward.get(&Symbol::External("explode".to_string())).unwrap().clone()
        }
    }
    pub fn lookup_symbol(&self, sym: &Symbol) -> SymbolDef {
        let this = self.0.borrow();
        this.forward.get(sym).unwrap_or_else(|| panic!("No symbol {:?}", sym)).clone()
    }
    pub fn add_global(&self, name: &Symbol, address: u64) -> SymbolDef {
        let mut this = self.0.borrow_mut();
        let def = SymbolDef::Global(address);
        assert!(this.forward.insert(name.clone(), def.clone()).is_none(), "duplicate {:?}", name);
        assert!(this.reverse.insert(address, name.clone()).is_none());
        def
    }
    pub fn add_thread_local(&self, name: &Symbol) -> SymbolDef {
        let mut this = self.0.borrow_mut();
        let key = this.next_thread_local_key;
        this.next_thread_local_key += 1;
        let def = SymbolDef::ThreadLocal(ThreadLocalKey(key));
        assert!(this.forward.insert(name.clone(), def.clone()).is_none(), "duplicate {:?}", name);
        def
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