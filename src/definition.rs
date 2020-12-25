use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use llvm_ir::module::ThreadLocalMode;

use crate::function::Func;
use crate::layout::Layout;
use crate::native;
use crate::symbols::{Symbol, ThreadLocalKey};
use crate::value::Value;

struct DefinitionsInner {
    functions: HashMap<u64, Rc<dyn Func>>,
    thread_local_inits: HashMap<ThreadLocalKey, (Layout, Value)>,
}

pub struct Definitions(RefCell<DefinitionsInner>);

impl Definitions {
    pub fn new() -> Self {
        Definitions(RefCell::new(DefinitionsInner {
            functions: HashMap::new(),
            thread_local_inits: HashMap::new(),
        }))
    }
    pub fn reverse_lookup_fun(&self, address: u64) -> Result<Rc<dyn Func>, String> {
        Ok(self.0.borrow()
            .functions
            .get(&address)
            .ok_or_else(|| format!("No such function {:?}", address))?
            .clone())
    }
    pub fn add_func(&self, address: u64, f: Rc<dyn Func>) {
        assert!(self.0.borrow_mut().functions.insert(address, f).is_none());
    }
    pub fn add_thread_local_init(&self, key: ThreadLocalKey, layout: Layout, value: Value) {
        assert!(self.0.borrow_mut().thread_local_inits.insert(key, (layout, value)).is_none());
    }
    pub fn thread_local_init(&self, key: ThreadLocalKey) -> (Layout, Value) {
        self.0.borrow().thread_local_inits.get(&key).unwrap().clone()
    }
}