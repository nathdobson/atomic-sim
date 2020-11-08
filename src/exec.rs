use crate::flow::{Ctx, Thread, Thunk, Node, Value};

use rand_xorshift::XorShiftRng;
use rand::SeedableRng;
use rand::Rng;
use std::rc::Rc;
use std::collections::{HashMap, BTreeMap};
use std::marker::PhantomData;
use std::fmt::{Debug, Formatter};
use std::fmt;
use llvm_ir::{Instruction, Type, Operand, Constant, IntPredicate, Name};
use llvm_ir::instruction::{Atomicity, InsertValue};
use std::ops::{Range, Deref};
use crate::layout::{Layout, align_to};
use llvm_ir::types::NamedStructDef;

#[derive(Debug)]
struct StoreLog {
    pos: u64,
    len: u64,
    value: Value,
}

#[derive(Debug)]
pub struct Alloc<'ctx> {
    len: u64,
    phantom: PhantomData<&'ctx ()>,
    stores: Vec<StoreLog>,
}

const HEAP_GUARD: u64 = 128;

#[derive(Debug)]
pub struct Memory<'ctx> {
    allocs: BTreeMap<u64, Alloc<'ctx>>,
    next: u64,
}

pub struct Process<'ctx> {
    ctx: &'ctx Ctx<'ctx>,
    threads: BTreeMap<usize, Thread<'ctx>>,
    next_threadid: usize,
    rng: XorShiftRng,
    memory: Memory<'ctx>,
    globals: HashMap<&'ctx Name, Value>,
    ptr_bits: u64,
}

impl<'ctx> Process<'ctx> {
    pub fn new(ctx: &'ctx Ctx) -> Self {
        let mut memory = Memory {
            allocs: BTreeMap::new(),
            next: 0,
        };
        let mut process = Process {
            ctx,
            threads: BTreeMap::new(),
            next_threadid: 0,
            rng: XorShiftRng::seed_from_u64(0),
            memory,
            globals: Default::default(),
            ptr_bits: 64,
        };
        process.initialize_globals();
        process
    }
    fn initialize_globals(&mut self) {
        for g in self.ctx.module.global_vars.iter() {
            let len = Value::new(self.ptr_bits, self.layout(&g.ty).size() as u128);
            let align = Value::new(self.ptr_bits, g.alignment as u128);
            self.globals.insert(&g.name, self.memory.alloc(len, align));
        }
    }
    pub fn add_thread(&mut self, main: &str) {
        let threadid = self.next_threadid;
        self.next_threadid += 1;
        self.threads.insert(threadid, Thread::new(self.ctx, main, threadid));
    }
    pub fn step(&mut self) -> bool {
        if self.threads.is_empty() {
            return false;
        }
        let index = self.rng.gen_range(0, self.threads.len());
        let (&threadid, thread) = self.threads.iter_mut().nth(index).unwrap();
        let decode_alive = thread.decode();
        let seq = match thread.thunks.iter().next() {
            None => {
                assert!(!decode_alive);
                self.threads.remove(&threadid);
                return true;
            }
            Some((seq, _)) => *seq,
        };
        let next = thread.thunks.remove(&seq).unwrap();
        let deps: Vec<Value> = next.deps.iter().map(|dep| dep.unwrap()).collect();
        println!("Exec {:?}", next.node);
        match &next.node {
            Node::Return(ret) => {
                for x in next.deps.iter() {
                    self.memory.free(x.unwrap());
                }
                if let Some(ret) = ret {
                    next.set(ret.unwrap())
                } else {
                    next.set(Value::from(()));
                }
            }
            Node::Constant(c) => { next.set(self.get_constant(c)) }
            Node::Store(store) => {
                let len = self.layout_of_operand(&store.value).size();
                self.memory.store(deps[0], len, deps[1], &store.atomicity);
                next.set(Value::new(0, 0));
            }
            Node::Load(load) => {
                let layout = self.layout(self.target_of(self.type_of(&load.address)));
                next.set(
                    self.memory.load(deps[0],
                                     layout.size(),
                                     &load.atomicity));
            }
            Node::Phi(phi) => todo!(),
            Node::Alloca(alloca) => {
                let len = Value::from(self.layout(&alloca.allocated_type).size());
                let align = Value::from(alloca.alignment);
                next.set(self.memory.alloc(len, align));
            }
            Node::Apply(instr) => {
                match instr {
                    Instruction::Add(_) => {
                        let (x, y) = (deps[0], deps[1]);
                        match (x.bits(), y.bits()) {
                            (64, 64) => next.set(Value::from(x.as_u64() + y.as_u64())),
                            bits => todo!("{:?}", bits)
                        }
                    }
                    Instruction::Xor(_) => {
                        let (x, y) = (deps[0], deps[1]);
                        match (x.bits(), y.bits()) {
                            (1, 1) => next.set(Value::from(x.as_bool() ^ y.as_bool())),
                            (64, 64) => next.set(Value::from(x.as_u64() ^ y.as_u64())),
                            bits => todo!("{:?}", bits)
                        }
                    }
                    Instruction::BitCast(bitcast) => {
                        next.set(deps[0]);
                    }
                    Instruction::ICmp(icmp) => {
                        let (x, y) = (deps[0], deps[1]);
                        match icmp.predicate {
                            IntPredicate::EQ => match (x.bits(), y.bits()) {
                                (64, 64) => next.set(Value::from(x.as_u64() == y.as_u64())),
                                bits => todo!("{:?}", bits),
                            }
                            pred => todo!("{:?}", pred),
                        }
                    }
                    Instruction::GetElementPtr(gep) => {
                        let mut ty = self.type_of(&gep.address);
                        let mut address = deps[0].as_u64();
                        for i in deps[1..].iter() {
                            let (offset2, ty2) = self.field(ty, i.as_u64());
                            address += offset2;
                            ty = ty2;
                        }

                        next.set(self.value_from_address(address));
                    }
                    Instruction::InsertValue(InsertValue { aggregate, indices, .. }) => {
                        let mut ty = self.type_of(&aggregate);
                        let mut offset = 0;
                        for i in indices {
                            let (offset2, ty2) = self.field(ty, *i as u64);
                            ty = ty2;
                            offset += offset2;
                        }
                        let mut aggregate = deps[0];
                        let element = deps[1];
                        let mut agg_bytes = aggregate.as_bytes();
                        let elem_bytes = element.as_bytes();
                        assert_eq!(element.bits() % 8, 0);
                        for i in 0..element.bits() / 8 {
                            agg_bytes[(offset + i) as usize] = elem_bytes[i as usize];
                        }
                        next.set(Value::with_bytes(aggregate.bits(), agg_bytes))
                    }
                    _ => todo!("{:?}", instr)
                }
            }
            Node::Intrinsic(call) => {
                match *call {
                    "llvm.dbg.declare" => {
                        next.set(Value::from(()))
                    }
                    "_ZN4core5panic8Location6caller17h4a6732fb1c79b809E" => {
                        next.set(Value::new(0, 0))
                    }
                    _ => todo!("{}", *call),
                }
            }
            Node::Metadata => {
                next.set(Value::from(()))
            }
        }
        if next.get().is_none() {
            panic!("Unfinished {:?}", next);
        }
        return true;
    }
    fn value_from_address(&self, x: u64) -> Value {
        match self.ptr_bits {
            32 => assert!(x <= u32::MAX as u64),
            64 => assert!(x <= u64::MAX),
            _ => todo!(),
        }
        Value::from(x)
    }
    fn get_constant(&mut self, c: &'ctx Constant) -> Value {
        match c {
            Constant::Int { bits, value } => {
                Value::new(*bits as u64, *value as u128)
            }
            Constant::BitCast(bitcast) => {
                self.get_constant(&*bitcast.operand)
            }
            Constant::GlobalReference { name, ty } => {
                *self.globals.get(name).unwrap_or_else(|| panic!("No global {:?}", name))
            }
            Constant::Undef(typ) => {
                Value::new(self.layout(typ).size() * 8, 0)
            }
            x => todo!("{:?}", x),
        }
    }
    fn layout_of_int(&self, bits: u64) -> Layout {
        Layout::from_size_align((bits / 8) as u64, (bits / 8) as u64)
    }
    fn layout_of_ptr(&self) -> Layout {
        Layout::from_size_align(self.ptr_bits / 8, self.ptr_bits / 8)
    }
    fn layout(&self, typ: &Type) -> Layout {
        match typ {
            Type::IntegerType { bits } => self.layout_of_int(*bits as u64),
            Type::PointerType { .. } => self.layout_of_ptr(),
            Type::StructType { element_types, is_packed } => {
                assert!(!is_packed);
                let mut layout = Layout::from_size_align(0, 1);
                for f in element_types {
                    layout = layout.extend(self.layout(&**f))
                }
                layout
            }
            Type::ArrayType { element_type, num_elements } => {
                let layout = self.layout(&*element_type);
                Layout::from_size_align(layout.size() * (*num_elements as u64), layout.align())
            }
            Type::NamedStructType { name } => {
                match self.ctx.module.types.named_struct_def(name)
                    .unwrap_or_else(|| panic!("Unknown struct {}", name)) {
                    NamedStructDef::Opaque => panic!("Cannot layout opaque {}", name),
                    NamedStructDef::Defined(ty) => {
                        self.layout(&*ty)
                    }
                }
            }
            _ => todo!("{:?}", typ),
        }
    }
    // fn layout_of_operand_target(&self, oper: &Operand) -> Layout {
    //     match oper {
    //         Operand::LocalOperand { name, ty } => {
    //             match &**ty {
    //                 Type::PointerType { pointee_type, addr_space } =>
    //                     self.layout(pointee_type),
    //                 x => todo!("{:?}", x),
    //             }
    //         }
    //         x => todo!("{:?}", x),
    //     }
    // }
    fn target_of(&self, ty: &'ctx Type) -> &'ctx Type {
        match ty {
            Type::PointerType { pointee_type, .. } => &*pointee_type,
            _ => todo!("{:?}", ty),
        }
    }
    fn type_of(&self, oper: &'ctx Operand) -> &'ctx Type {
        match oper {
            Operand::LocalOperand { name, ty } => {
                &**ty
            }
            Operand::ConstantOperand(c) => {
                match &**c {
                    Constant::Int { bits, value } => {
                        match bits {
                            0 => &Type::IntegerType { bits: 0 },
                            _ => todo!("{}", bits)
                        }
                    }
                    Constant::Undef(ty) => {
                        &**ty
                    }
                    x => todo!("{:?}", x),
                }
            }
            x => todo!("{:?}", x),
        }
    }
    fn layout_of_operand(&self, oper: &Operand) -> Layout {
        match oper {
            Operand::LocalOperand { name, ty } => {
                self.layout(ty)
            }
            Operand::ConstantOperand(c) => {
                match &**c {
                    Constant::Int { bits, value } => self.layout_of_int(*bits as u64),
                    x => todo!("{:?}", x),
                }
            }
            x => todo!("{:?}", x),
        }
    }
    fn field(&self, ty: &'ctx Type, index: u64) -> (u64, &'ctx Type) {
        match ty {
            Type::PointerType { pointee_type, .. } => {
                (index * self.layout(&*pointee_type).size(), pointee_type)
            }
            Type::StructType { element_types, .. } => {
                let mut offset = Layout::from_size_align(0, 1);
                for field in element_types.iter().take(index as usize) {
                    offset = offset.extend(self.layout(&**field))
                }
                let final_align = self.layout(&*element_types[index as usize]).align();
                (align_to(offset.size(), final_align), &element_types[index as usize])
            }
            ty => todo!("{:?}", ty),
        }
    }
}

impl<'ctx> Memory<'ctx> {
    fn free(&mut self, ptr: Value) {}
    fn alloc(&mut self, len: Value, align: Value) -> Value {
        let len = len.as_u64();
        let align = align.as_u64();
        self.next = (self.next.wrapping_sub(1) | (align - 1)).wrapping_add(1);
        let result = self.next;
        self.next = self.next + len + HEAP_GUARD;
        self.allocs.insert(result, Alloc { len, phantom: PhantomData, stores: vec![] });
        Value::from(result)
    }
    fn find_alloc<'a>(&'a mut self, ptr: Value) -> &'a mut Alloc<'ctx> where 'ctx: 'a {
        self.allocs.range_mut(..=ptr.as_u64()).next_back().unwrap().1
    }
    fn store(&mut self, ptr: Value, len: u64, value: Value, atomicity: &'ctx Option<Atomicity>) {
        self.find_alloc(ptr).stores.push(
            StoreLog {
                pos: ptr.as_u64(),
                len,
                value,
            }
        );
    }
    fn intersects(r1: Range<u64>, r2: Range<u64>) -> bool {
        r1.contains(&r2.start) || r2.contains(&r1.start)
    }
    fn load(&mut self, ptr: Value, len: u64, atomicity: &'ctx Option<Atomicity>) -> Value {
        self.find_alloc(ptr).stores.iter().filter(|store| {
            let ptr = ptr.as_u64();
            let r1 = (store.pos..store.pos + store.len);
            let r2 = (ptr..ptr + len);
            if Self::intersects(r1.clone(), r2.clone()) {
                assert_eq!(r1, r2);
                true
            } else { false }
        }).last().unwrap().value
    }
}

impl<'ctx> Debug for Process<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("threads", &self.threads)
            .field("memory", &self.memory)
            .finish()
    }
}