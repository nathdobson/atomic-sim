use rand_xorshift::XorShiftRng;
use rand::SeedableRng;
use rand::Rng;
use std::rc::Rc;
use std::collections::{HashMap, BTreeMap};
use std::marker::PhantomData;
use std::fmt::{Debug, Formatter};
use std::{fmt, mem};
use llvm_ir::{Instruction, Type, Operand, Constant, IntPredicate, Name, Terminator, constant, TypeRef};
use llvm_ir::instruction::{Atomicity, InsertValue, SExt, ZExt, AtomicRMW, RMWBinOp, Select, Trunc, PtrToInt, ExtractValue, IntToPtr, CmpXchg};
use std::ops::{Range, Deref, DerefMut};
use crate::layout::{Layout, align_to, AggrLayout, Packing};
use llvm_ir::types::NamedStructDef;
use llvm_ir::constant::BitCast;
use std::borrow::{Cow};
use crate::value::{Value, add_u64_i64};
use std::convert::TryInto;
use std::panic::UnwindSafe;
use crate::symbols::{Symbol, SymbolTable, ModuleId};
use std::time::{Instant, Duration};
use crate::data::Thunk;
use std::cell::RefCell;
use crate::function::Func;
use crate::memory::Memory;
use crate::thread::{ThreadId, Thread};

pub struct ProcessInner {
    functions: HashMap<Symbol, Rc<dyn Func>>,
    ptr_bits: u64,
    page_size: u64,
    rng: XorShiftRng,
    threads: BTreeMap<ThreadId, Thread>,
    next_threadid: usize,
    symbols: SymbolTable,
    memory: Memory,
}

#[derive(Clone)]
pub struct Process(Rc<RefCell<ProcessInner>>);

pub struct ProcessScope(Process);

impl Process {
    pub fn new() -> (Self, ProcessScope) {
        let result = Process(Rc::new(RefCell::new(ProcessInner {
            functions: HashMap::new(),
            ptr_bits: 64,
            page_size: 4096,
            threads: BTreeMap::new(),
            next_threadid: 0,
            rng: XorShiftRng::seed_from_u64(0),
            symbols: SymbolTable::new(),
            memory: Memory::new(),
        })));
        (result.clone(), ProcessScope(result))
    }
    pub fn ptr_bits(&self) -> u64 {
        self.0.borrow().ptr_bits
    }
    pub fn add_thread(&self, process: Process, main: Symbol) {
        let threadid = {
            let mut this = self.0.borrow_mut();
            let threadid = ThreadId(this.next_threadid);
            this.next_threadid += 1;
            threadid
        };
        let thread = Thread::new(process.clone(), threadid, main,
                                 &[Value::new(32, 0),
                                     Value::new(process.ptr_bits(), 0)]);
        self.0.borrow_mut().threads.insert(threadid,
                                           thread);
    }
    pub fn step(&self) -> bool {
        let (threadid, thread) = {
            let mut this = self.0.borrow_mut();
            let this = this.deref_mut();
            if this.threads.is_empty() {
                return false;
            }
            let index = this.rng.gen_range(0, this.threads.len());
            let thread = this.threads.iter_mut().nth(index).unwrap().1.clone();
            (ThreadId(index), thread)
        };
        if !thread.step() {
            self.0.borrow_mut().threads.remove(&threadid);
        }
        true
    }
    pub fn free(&self, tid: ThreadId, ptr: &Value) {
        self.0.borrow_mut().memory.free(ptr)
    }
    pub fn alloc(&self, tid: ThreadId, layout: Layout) -> Value {
        self.0.borrow_mut().memory.alloc(layout)
    }
    pub fn realloc(&self, tid: ThreadId, old: &Value, old_layout: Layout, new_layout: Layout) -> Value {
        let new = self.alloc(tid, new_layout);
        self.memcpy(tid, &new, old, new_layout.bytes().min(old_layout.bytes()));
        self.free(tid, old);

        new
    }
    pub fn store(&self, threadid: ThreadId, ptr: &Value, value: &Value, atomicity: Option<Atomicity>) {
        self.0.borrow_mut().memory.store(threadid, ptr, value, atomicity);
    }
    pub fn load(&self, threadid: ThreadId, ptr: &Value, layout: Layout, atomicity: Option<Atomicity>) -> Value {
        self.0.borrow_mut().memory.load(ptr, layout, atomicity)
    }
    pub fn memcpy(&self, threadid: ThreadId, dst: &Value, src: &Value, len: u64) {
        if len > 0 {
            let value = self.load(threadid, &src, Layout::from_bytes(len, 1), None);
            self.0.borrow_mut().memory.store(threadid, &dst, &value, None);
        }
    }
    pub fn debug_info(&self, ptr: &Value) -> String {
        let this = self.0.borrow_mut();
        format!("{:?} {:?} {:?}",
                ptr,
                this.symbols.try_reverse_lookup(ptr),
                this.memory.debug_info(ptr))
    }
    // pub fn new(modules: &'ctx [Module], native: Vec<Rc<dyn 'ctx + Func<'ctx>>>) -> Ctx<'ctx> {
    //     let mut functions: HashMap<Symbol, Rc<dyn 'ctx + Func<'ctx>>> = HashMap::new();
    //     for (mi, module) in modules.iter().enumerate() {
    //         let ectx = EvalCtx { module: Some(mi) };
    //         for function in module.functions.iter() {
    //             let symbol = Symbol::new(function.linkage, mi, &function.name);
    //             let compiled = Rc::new(InterpFunc::new(ectx, symbol.clone(), function));
    //             functions.insert(symbol, compiled);
    //         }
    //     }
    //     for native in native.into_iter() {
    //         functions.insert(Symbol::External(native.name().to_string()), native);
    //     }
    //     let mut ctx = Ctx {
    //         modules,
    //         functions,
    //         ptr_bits: 64,
    //         page_size: 4096,
    //         symbols: SymbolTable::new(),
    //         image: Memory::new(),
    //     };
    //     ctx.initialize_globals();
    //     ctx
    // }
    // pub fn named_struct_def(&self, x: &str) -> Option<&NamedStructDef> {
    //     self.modules.iter().find_map(|m| m.types.named_struct_def(x))
    // }
    // pub fn struct_of(&self, fields: Vec<TypeRef>, is_packed: bool) -> TypeRef {
    //     self.modules[0].types.struct_of(fields, is_packed)
    // }
    // pub fn array_of(&self, elem: TypeRef, num_elements: u64) -> TypeRef {
    //     self.modules[0].types.array_of(elem, num_elements as usize)
    // }
    // pub fn vector_of(&self, elem: TypeRef, num_elements: u64) -> TypeRef {
    //     self.modules[0].types.vector_of(elem, num_elements as usize)
    // }
    // pub fn int(&self, bits: u64) -> TypeRef {
    //     self.modules[0].types.int(bits as u32)
    // }
    // pub fn pointer_to(&self, ty: TypeRef) -> TypeRef {
    //     self.modules[0].types.pointer_to(ty)
    // }
    pub fn value_from_address(&self, x: u64) -> Value {
        let this = self.0.borrow_mut();
        match this.ptr_bits {
            32 => assert!(x <= u32::MAX as u64),
            64 => assert!(x <= u64::MAX),
            _ => todo!(),
        }
        Value::from(x)
    }
    pub fn null(&self) -> Value {
        self.value_from_address(0)
    }
    // pub fn aggregate_zero(&self, ty: &TypeRef) -> Value {
    //     Value::zero(self.layout(ty))
    // }
    pub fn layout_of_ptr(&self) -> Layout {
        let this = self.0.borrow_mut();
        Layout::from_bits(this.ptr_bits, this.ptr_bits)
    }
    // pub fn layout(&self, typ: &TypeRef) -> Layout {
    //     match &**typ {
    //         Type::IntegerType { bits } => Layout::of_int(*bits as u64),
    //         Type::PointerType { .. } => self.layout_of_ptr(),
    //         Type::StructType { element_types, is_packed } => {
    //             AggrLayout::new(Packing::from(*is_packed), element_types.iter().map(|t| self.layout(t))).layout()
    //         }
    //         Type::VectorType { element_type, num_elements } => {
    //             let layout = self.layout(&*element_type).pad_to_align();
    //             layout.repeat(Packing::None, *num_elements as u64)
    //         }
    //         Type::ArrayType { element_type, num_elements } => {
    //             let layout = self.layout(&*element_type).pad_to_align();
    //             layout.repeat(Packing::None, *num_elements as u64)
    //         }
    //         Type::NamedStructType { name } => {
    //             self.layout(self.type_of_struct(name))
    //         }
    //         _ => todo!("{:?}", typ),
    //     }
    // }
    // pub fn type_of_struct<'a>(&'a self, name: &'a str) -> &'a TypeRef {
    //     match self.named_struct_def(name)
    //         .unwrap_or_else(|| panic!("Unknown struct {}", name)) {
    //         NamedStructDef::Opaque => panic!("Cannot layout opaque {}", name),
    //         NamedStructDef::Defined(ty) => {
    //             ty
    //         }
    //     }
    // }
    pub fn target_of(&self, ty: &TypeRef) -> TypeRef {
        match &**ty {
            Type::PointerType { pointee_type, .. } => pointee_type.clone(),
            _ => todo!("{:?}", ty),
        }
    }
    // pub fn type_of(&self, oper: &'ctx Operand) -> TypeRef {
    //     match oper {
    //         Operand::LocalOperand { ty, .. } => {
    //             ty.clone()
    //         }
    //         Operand::ConstantOperand(c) => {
    //             self.get_constant(ectx, c).0
    //         }
    //         x => todo!("{:?}", x),
    //     }
    // }
//    }
    // pub fn field_bit<'a>(&'a self, ty: &'a Type, index: i64) -> (TypeRef, i64) {
    //     match ty {
    //         Type::PointerType { pointee_type, .. } => {
    //             (pointee_type.clone(), index * (self.layout(&*pointee_type).bits() as i64))
    //         }
    //         Type::StructType { element_types, is_packed } => {
    //             let layout = AggrLayout::new(
    //                 (*is_packed).into(),
    //                 element_types
    //                     .iter()
    //                     .map(|t| self.layout(t)));
    //             let offset_bits = layout.bit_offset(index as usize);
    //             (element_types[index as usize].clone(), offset_bits as i64)
    //         }
    //         Type::ArrayType { element_type, num_elements } => {
    //             let layout = self.layout(&element_type);
    //             let stride = align_to(layout.bits(), layout.bit_align());
    //             (element_type.clone(), index * stride as i64)
    //         }
    //         Type::NamedStructType { name } => {
    //             let ty: &'a TypeRef = self.type_of_struct(name);
    //             self.field_bit(ty, index)
    //         }
    //         Type::VectorType { element_type, num_elements } => {
    //             (element_type.clone(), index * (self.layout(&element_type).bits() as i64))
    //         }
    //         ty => todo!("{:?}", ty),
    //     }
    // }
    // pub fn count<'a>(&'a self, ty: &'a Type) -> u64 {
    //     match ty {
    //         Type::StructType { element_types, is_packed } => {
    //             element_types.len() as u64
    //         }
    //         Type::ArrayType { element_type, num_elements } => {
    //             *num_elements as u64
    //         }
    //         Type::NamedStructType { name } => {
    //             let ty: &'a TypeRef = self.type_of_struct(name);
    //             self.count(ty)
    //         }
    //         Type::VectorType { element_type, num_elements } => {
    //             *num_elements as u64
    //         }
    //         ty => todo!("{:?}", ty),
    //     }
    // }
    pub fn reverse_lookup(&self, address: &Value) -> Symbol {
        self.0.borrow().symbols.reverse_lookup(address)
    }
    pub fn reverse_lookup_fun(&self, address: &Value) -> Rc<dyn Func> {
        let name = self.reverse_lookup(address);
        self.0.borrow().functions.get(&name).unwrap_or_else(|| panic!("No such function {:?}", name)).clone()
    }
    pub fn try_reverse_lookup(&self, address: &Value) -> Option<Symbol> {
        self.0.borrow().symbols.try_reverse_lookup(address)
    }
    pub fn lookup(&self, moduleid: Option<ModuleId>, name: &str) -> Value {
        self.0.borrow().symbols.lookup(moduleid, name)
    }
    pub fn lookup_symbol(&self, sym: &Symbol) -> Value {
        self.0.borrow().symbols.lookup_symbol(sym)
    }
    // pub fn extract_value(&self, ty: &TypeRef, v: &Value, indices: impl Iterator<Item=i64>) -> Value {
    //     let (ty2, offset) = self.offset_bit(&ty, indices);
    //     let layout = self.layout(&ty2);
    //     v.extract_bits(offset, layout)
    // }
    // pub fn insert_value(&self, ty: &TypeRef, aggregate: &mut Value, element: &Value, indices: impl Iterator<Item=i64>) {
    //     let (ty2, offset) = self.offset_bit(&ty, indices);
    //     let layout = self.layout(&ty2);
    //     aggregate.insert_bits(offset, element)
    // }
    pub fn add_symbol(&self, symbol: Symbol, layout: Layout) -> Value {
        let mut this = self.0.borrow_mut();
        let this = this.deref_mut();
        this.symbols.add_symbol(&mut this.memory, symbol, layout)
    }
    pub fn add_alias(&self, symbol: Symbol, value: Value) {
        let mut this = self.0.borrow_mut();
        let this = this.deref_mut();
        this.symbols.add_alias(symbol, value);
    }
}


impl Debug for Process {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("threads", &self.0.borrow().threads)
            .finish()
    }
}

impl Drop for ProcessScope {
    fn drop(&mut self) {
        let mut this = self.0.0.borrow_mut();
        this.threads.clear();
    }
}
