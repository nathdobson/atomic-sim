use crate::flow::{Thread, Thunk, Node};

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
use std::ops::{Range, Deref};
use crate::layout::{Layout, align_to, AggrLayout};
use llvm_ir::types::NamedStructDef;
use llvm_ir::constant::BitCast;
use crate::memory::Symbol;
use std::borrow::Cow;
use crate::value::{Value, add_u64_i64};
use crate::memory::Memory;
use std::convert::TryInto;
use crate::ctx::{Ctx, EvalCtx, ThreadCtx};
use std::panic::UnwindSafe;


pub struct Process<'ctx> {
    pub ctx: &'ctx Ctx<'ctx>,
    pub threads: BTreeMap<usize, Thread<'ctx>>,
    pub next_threadid: usize,
    pub rng: XorShiftRng,
    pub memory: Memory<'ctx>,
}

fn str_of_name(name: &Name) -> &str {
    match name {
        Name::Name(name) => &***name,
        Name::Number(_) => panic!(),
    }
}

impl<'ctx> Process<'ctx> {
    pub fn new(ctx: &'ctx Ctx) -> Self {
        let mut process = Process {
            ctx,
            threads: BTreeMap::new(),
            next_threadid: 0,
            rng: XorShiftRng::seed_from_u64(0),
            memory: Memory::new(ctx),
        };
        process.initialize_globals();
        process
    }
    pub fn ctx(&self) -> &'ctx Ctx<'ctx> {
        self.ctx
    }
    fn initialize_globals(&mut self) {
        let func_layout = Layout::from_bytes(256, 1);
        for (name, _) in self.ctx.native.iter() {
            self.memory.add_symbol(*name, func_layout);
        }
        let mut inits = vec![];
        for (mi, module) in self.ctx.modules.iter().enumerate() {
            for f in module.functions.iter() {
                let loc = self.memory.add_symbol(Symbol::new(f.linkage, mi, &f.name), func_layout);
            }
            for g in module.global_vars.iter() {
                let symbol = Symbol::new(g.linkage, mi, str_of_name(&g.name));
                let mut layout = self.ctx.layout(&self.target_of(&g.ty));
                layout = Layout::from_bits(layout.bits(), layout.bit_align().max(8 * g.alignment as u64));
                let loc = self.memory.add_symbol(symbol, layout);
                inits.push((mi, symbol, layout, g, loc));
            }
        }
        for (mi, symbol, layout, g, loc) in inits {
            let (ty, value) =
                self.get_constant(EvalCtx { module: Some(mi) }, g.initializer.as_ref().unwrap());
            assert_eq!(ty, self.target_of(&g.ty), "{} == {}", ty, g.ty);
            self.memory.store(&loc, &value, None);
        }
        for (mi, module) in self.ctx.modules.iter().enumerate() {
            for g in module.global_vars.iter() {
                let symbol = Symbol::new(g.linkage, mi, str_of_name(&g.name));
            }
        }
    }
    pub fn add_thread(&mut self, main: Symbol<'ctx>) {
        let threadid = self.next_threadid;
        self.next_threadid += 1;
        self.threads.insert(threadid,
                            Thread::new(self.ctx, main, threadid,
                                        &[Value::new(32, 0),
                                            Value::new(self.ctx.ptr_bits, 0)]));
    }
    pub fn step(&mut self) -> bool {
        if self.threads.is_empty() {
            return false;
        }
        let index = self.rng.gen_range(0, self.threads.len());
        let (&threadid, thread) = self.threads.iter_mut().nth(index).unwrap();
        let decode_alive = thread.decode(&self.memory);
        let seq = match thread.thunks.iter().next() {
            None => {
                if decode_alive {
                    panic!("Decoding at {:#?}", thread);
                }
                self.threads.remove(&threadid);
                return true;
            }
            Some((seq, _)) => *seq,
        };
        let next = thread.thunks.remove(&seq).unwrap();
        let deps: Vec<&Value> = next.deps.iter().map(|dep| dep.unwrap()).collect();
        let tctx = ThreadCtx { ectx: next.ectx, threadid: threadid };
        match &next.node {
            Node::Return(ret, allocs) => {
                for x in allocs {
                    self.free(tctx, x.unwrap());
                }
                if let Some(ret) = ret {
                    next.set(ret.unwrap().clone())
                } else {
                    next.set(Value::from(()));
                }
            }
            Node::Constant(c) => { next.set(self.get_constant(tctx.ectx, c).1) }
            Node::Instr(instr) => {
                match instr {
                    Instruction::Store(store) => {
                        self.memory.store(&deps[0], deps[1], store.atomicity.as_ref());
                        next.set(Value::new(0, 0));
                    }
                    Instruction::Load(load) => {
                        let ty = self.target_of(&self.type_of(next.ectx, &load.address));
                        let layout = self.ctx.layout(&ty);
                        let value = self.memory.load(deps[0],
                                                     layout,
                                                     load.atomicity.as_ref());
                        next.set(value.clone());
                    }
                    Instruction::Phi(_phi) => todo!(),
                    Instruction::Alloca(alloca) => {
                        let layout = self.ctx.layout(&alloca.allocated_type);
                        next.set(self.memory.alloc(layout));
                    }
                    Instruction::Add(_) => next.set(deps[0] + deps[1]),
                    Instruction::Sub(_) => next.set(deps[0] - deps[1]),
                    Instruction::Xor(_) => next.set(deps[0] ^ deps[1]),
                    Instruction::And(_) => next.set(deps[0] & deps[1]),
                    Instruction::URem(_) => next.set(deps[0] % deps[1]),
                    Instruction::UDiv(_) => next.set(deps[0] / deps[1]),
                    Instruction::SRem(_) => next.set(deps[0].srem(deps[1])),
                    Instruction::SDiv(_) => next.set(deps[0].sdiv(deps[1])),
                    Instruction::Mul(_) => next.set(deps[0] * deps[1]),
                    Instruction::LShr(_) => next.set(deps[0] >> deps[1]),
                    Instruction::Shl(_) => next.set(deps[0] << deps[1]),
                    Instruction::SExt(SExt { to_type, .. }) =>
                        next.set(deps[0].sext(self.ctx.layout(to_type).bits())),
                    Instruction::ZExt(ZExt { to_type, .. }) =>
                        next.set(Value::new(self.ctx.layout(to_type).bits(), deps[0].as_u128())),
                    Instruction::BitCast(bitcast) =>
                        next.set(deps[0].clone()),
                    Instruction::PtrToInt(PtrToInt { to_type, .. }) =>
                        next.set(deps[0].clone()),
                    Instruction::IntToPtr(IntToPtr { to_type, .. }) =>
                        next.set(deps[0].clone()),

                    Instruction::ICmp(icmp) => {
                        let (x, y) = (deps[0], deps[1]);
                        next.set(Value::from(match icmp.predicate {
                            IntPredicate::EQ => x.as_u128() == y.as_u128(),
                            IntPredicate::NE => x.as_u128() != y.as_u128(),
                            IntPredicate::ULE => x.as_u128() <= y.as_u128(),
                            IntPredicate::ULT => x.as_u128() < y.as_u128(),
                            IntPredicate::UGE => x.as_u128() >= y.as_u128(),
                            IntPredicate::UGT => x.as_u128() > y.as_u128(),
                            IntPredicate::SGT => x.as_i128() > y.as_i128(),
                            IntPredicate::SGE => x.as_i128() >= y.as_i128(),
                            IntPredicate::SLT => x.as_i128() < y.as_i128(),
                            IntPredicate::SLE => x.as_i128() <= y.as_i128(),
                            pred => todo!("{:?}", pred),
                        }))
                    }
                    Instruction::GetElementPtr(gep) => {
                        let ty = &self.type_of(next.ectx, &gep.address);
                        let (_, offset) = self.offset_of(ty,
                                                         deps[1..].iter().map(|v| v.as_i64()));
                        next.set(self.ctx.value_from_address(add_u64_i64(deps[0].as_u64(), offset)));
                    }
                    Instruction::InsertValue(InsertValue { aggregate, indices, .. }) => {
                        let ty = self.type_of(next.ectx, aggregate);
                        let (_, offset) = self.offset_of(&ty, indices.iter().map(|i| *i as i64));
                        let offset = offset as usize;
                        let mut aggregate = deps[0].clone();
                        let element = deps[1];
                        let element = element.bytes();
                        aggregate.bytes_mut()[offset..(offset + element.len())].copy_from_slice(element);
                        next.set(aggregate)
                    }

                    Instruction::AtomicRMW(AtomicRMW {
                                               address,
                                               atomicity,
                                               operation, ..
                                           }) => {
                        let ty = self.target_of(&self.type_of(next.ectx, address));
                        let layout = self.ctx.layout(&ty);
                        let current = self.memory.load(deps[0], layout, Some(atomicity)).clone();
                        let new = match operation {
                            RMWBinOp::Add => &current + deps[1],
                            RMWBinOp::Sub => &current - deps[1],
                            RMWBinOp::Xchg => deps[1].clone(),
                            RMWBinOp::And => &current & deps[1],
                            RMWBinOp::Or => &current | deps[1],
                            RMWBinOp::Xor => &current ^ deps[1],
                            _ => todo!("{:?}", operation),
                        };
                        self.memory.store(deps[0], &new, Some(atomicity));
                        next.set(current.clone());
                    }
                    Instruction::Select(Select { .. }) => {
                        if deps[0].unwrap_bool() {
                            next.set(deps[1].clone());
                        } else {
                            next.set(deps[2].clone());
                        }
                    }
                    Instruction::Trunc(Trunc { to_type, .. }) => {
                        match &**to_type {
                            Type::IntegerType { bits } => {
                                next.set(deps[0].truncate(*bits as u64));
                            }
                            _ => unreachable!("{:?}", to_type),
                        }
                    }
                    Instruction::ExtractValue(ExtractValue { aggregate, indices, dest, .. }) => {
                        let ty = self.type_of(next.ectx, aggregate);
                        let (ty2, offset) = self.offset_of(&ty, indices.iter().map(|i| *i as i64));
                        let layout = self.ctx.layout(&ty2);
                        let offset = offset as usize;
                        let bytes = &deps[0].bytes()[offset..offset + layout.bytes() as usize];
                        next.set(Value::from_bytes(bytes, layout));
                    }
                    Instruction::CmpXchg(CmpXchg { address, volatile, atomicity, failure_memory_ordering, .. }) => {
                        let ty = self.target_of(&self.type_of(next.ectx, address));
                        let layout = self.ctx.layout(&ty);
                        let (address, expected, replacement) = (deps[0], deps[1], deps[2]);
                        let old = self.memory.load(address, layout, Some(atomicity));
                        let success = &old == expected;
                        if success {
                            self.memory.store(address, replacement, Some(atomicity));
                        }
                        next.set(Value::aggregate(vec![old.clone(), Value::from(success)].into_iter(), false))
                    }

                    _ => todo!("{:?}", instr)
                }
            }
            Node::Native(call) => {
                next.set((call.imp)(self, tctx, deps.as_slice()));
            }
            Node::Metadata => {
                next.set(Value::from(()))
            }
            Node::Value(value) => {
                next.set(value.clone());
            }
        }
        if next.get().is_none() {
            panic!("Unfinished {:?}", next);
        } else {
            //println!("{:?}", next);
        }
        return true;
    }

    fn target_of(&self, ty: &TypeRef) -> TypeRef {
        match &**ty {
            Type::PointerType { pointee_type, .. } => pointee_type.clone(),
            _ => todo!("{:?}", ty),
        }
    }


}


impl<'ctx> Debug for Process<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("threads", &self.threads)
            //.field("memory", &self.memory)
            .finish()
    }
}
