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

#[derive(Copy, Clone, Debug, Ord, Eq, PartialEq, PartialOrd, Hash)]
pub struct ThreadLocalKey(usize);

pub struct SymbolTable {
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
        SymbolTable {
            forward: HashMap::new(),
            reverse: HashMap::new(),
            next_thread_local_key: 0,
        }
    }
    pub fn add_symbol(&mut self, memory: &mut Memory, name: Symbol, mode: ThreadLocalMode, layout: Layout) -> SymbolDef {
        let def = match mode {
            ThreadLocalMode::NotThreadLocal => {
                let address = memory.alloc(layout).as_u64();
                assert!(self.reverse.insert(address, name.clone()).is_none());
                SymbolDef::Global(address)
            }
            _ => {
                let key = self.next_thread_local_key;
                self.next_thread_local_key += 1;
                SymbolDef::ThreadLocal(ThreadLocalKey(key))
            }
        };
        assert!(self.forward.insert(name.clone(), def.clone()).is_none(), "duplicate {:?}", name);
        def
    }
    pub fn add_alias(&mut self, name: Symbol, address: Value) {
        let def = SymbolDef::Global(address.as_u64());
        match self.forward.entry(name.clone()) {
            Entry::Occupied(occupied) => { assert_eq!(occupied.get(), &def, "{:?}", name); }
            Entry::Vacant(vacant) => { vacant.insert(def); }
        }
    }
    pub fn reverse_lookup(&self, address: &Value) -> Symbol {
        self.reverse.get(&address.as_u64()).unwrap_or_else(|| panic!("No symbol at {:?}", address)).clone()
    }
    pub fn try_reverse_lookup(&self, address: &Value) -> Option<Symbol> {
        self.reverse.get(&address.as_u64()).cloned()
    }
    pub fn lookup(&self, module: Option<ModuleId>, name: &str) -> SymbolDef {
        if let Some(module) = module {
            if let Some(internal) = self.forward.get(&Symbol::Internal(module, name.to_string())) {
                return internal.clone();
            }
        }
        if let Some(external) = self.forward.get(&Symbol::External(name.to_string())) {
            external.clone()
        } else {
            println!("No symbol named {:?}", name);
            self.forward.get(&Symbol::External("explode".to_string())).unwrap().clone()
        }
    }
    pub fn lookup_symbol(&self, sym: &Symbol) -> SymbolDef {
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