use llvm_ir::{Function, BasicBlock, Name, Instruction, IntPredicate, Terminator, Operand, DebugLoc, Type, HasDebugLoc, TypeRef};
use std::collections::HashMap;
use crate::ctx::{EvalCtx, Ctx};
use crate::function::{Func};
use crate::data::{DataFlow, Thunk};
use std::rc::Rc;
use futures::future::LocalBoxFuture;
use llvm_ir::instruction::{RMWBinOp, Sub, Mul, UDiv, SDiv, URem, SRem, Add, And, Or, Shl, LShr, AShr, ICmp, Xor, SExt, ZExt, Trunc, PtrToInt, IntToPtr, BitCast, InsertValue, AtomicRMW, Select, ExtractValue, CmpXchg, Fence, InlineAssembly, ExtractElement, InsertElement, ShuffleVector};
use either::Either;
use std::{fmt, iter};
use futures::{Future, FutureExt};
use crate::value::{Value, add_u64_i64};
use llvm_ir::function::ParameterAttribute;
use std::fmt::Debug;
use crate::backtrace::{Backtrace, BacktraceFrame};
use crate::flow::FlowCtx;
use crate::compute::ComputeCtx;
use crate::symbols::Symbol;
use crate::layout::{AggrLayout, Layout, Packing};
use crate::arith::{BinOp, scast, ucast};

#[derive(Debug)]
pub struct InterpFunc<'ctx> {
    pub sym: Symbol,
    pub src: &'ctx Function,
    pub ectx: EvalCtx,
}

impl<'ctx> InterpFunc<'ctx> {
    pub fn new(ectx: EvalCtx, sym: Symbol, fun: &'ctx Function) -> Self {
        InterpFunc {
            sym,
            src: &fun,
            ectx: ectx,
        }
    }
}


struct InterpFrame<'ctx> {
    //pub ctx: Rc<Ctx<'ctx>>,
    pub fp: u64,
    pub fun: &'ctx Function,
    pub origin: Option<&'ctx Name>,
    pub temps: HashMap<&'ctx Name, Thunk<'ctx>>,
    pub allocs: Vec<Thunk<'ctx>>,
    pub result: Option<&'ctx Name>,
    pub blocks: HashMap<&'ctx Name, &'ctx BasicBlock>,
    pub ectx: EvalCtx,
}

enum DecodeResult<'ctx> {
    Jump(&'ctx Name),
    Return(Thunk<'ctx>),
}

impl<'ctx> Func<'ctx> for InterpFunc<'ctx> {
    fn name(&self) -> &'ctx str {
        self.src.name.as_str()
    }

    fn debugloc(&self) -> Option<&'ctx DebugLoc> {
        self.src.debugloc.as_ref()
    }

    fn call_imp<'flow>(&'flow self, flow: &'flow FlowCtx<'ctx, 'flow>, args: &'flow [Thunk<'ctx>]) -> LocalBoxFuture<'flow, Thunk<'ctx>> {
        Box::pin(InterpFrame::call(flow, self.ectx, flow.ctx().lookup_symbol(&self.sym).as_u64(), self.src, args.to_vec()))
    }
}

impl<'ctx> InterpFrame<'ctx> {
    pub async fn call<'flow>(flow: &'flow FlowCtx<'ctx, 'flow>,
                             ectx: EvalCtx,
                             fp: u64,
                             fun: &'ctx Function,
                             params: Vec<Thunk<'ctx>>) -> Thunk<'ctx> {
        let temps =
            fun.parameters.iter()
                .zip(params.into_iter())
                .map(|(n, v)| (&n.name, v))
                .collect();
        let blocks =
            fun.basic_blocks.iter()
                .map(|block| (&block.name, block))
                .collect();
        let mut frame = InterpFrame {
            //ctx: flow.ctx(),
            fp,
            fun,
            origin: None,
            temps,
            allocs: vec![],
            result: None,
            blocks,
            ectx,
        };
        frame.decode(flow).await
    }
    pub fn decode<'flow>(&'flow mut self, flow: &'flow FlowCtx<'ctx, 'flow>) -> impl Future<Output=Thunk<'ctx>> + 'flow {
        Box::pin(async move { self.decode_impl(flow).await })
    }
    pub async fn decode_impl(&mut self, flow: &FlowCtx<'ctx, '_>) -> Thunk<'ctx> {
        let mut block = self.fun.basic_blocks.get(0).unwrap();
        loop {
            let mut iter = block.instrs.iter().peekable();
            let mut phis = vec![];
            while let Some(Instruction::Phi(phi)) = iter.peek() {
                iter.next();
                let (oper, _) =
                    phi.incoming_values.iter()
                        .find(|(_, name)| Some(name) == self.origin)
                        .unwrap_or_else(|| panic!("No target {:?} in {:?}", self.origin, self.fun.name));
                let deps = vec![self.get_temp(&flow, oper).await];
                phis.push((phi, deps));
            }
            for (phi, deps) in phis {
                self.add_thunk(flow, format!("{}", phi), Some(&phi.dest), deps, |comp, args| args[0].clone()).await;
            }
            for instr in iter {
                self.decode_instr(&flow.with_frame(BacktraceFrame { ip: self.fp, ..BacktraceFrame::default() }), instr).await;
            }
            match self.decode_term(&flow.with_frame(BacktraceFrame { ip: self.fp, ..BacktraceFrame::default() }), &block.term).await {
                DecodeResult::Jump(name) => {
                    self.origin = Some(&block.name);
                    block = *self.blocks.get(name).unwrap();
                }
                DecodeResult::Return(result) => return result,
            };
        }
    }
    async fn decode_instr<'a>(&'a mut self, flow: &'a FlowCtx<'ctx, '_>, instr: &'ctx Instruction) {
        let name = format!("{} {:?}", instr, instr.get_debug_loc());
        match instr {
            Instruction::Call(call) => {
                self.decode_call(flow, &call.function, &call.arguments, call.dest.as_ref()).await;
            }
            Instruction::Alloca(alloca) => {
                let num = self.get_temp(flow, &alloca.num_elements).await;
                let layout = flow.ctx().layout(&alloca.allocated_type);
                let thunk = self.add_thunk(
                    flow,
                    name,
                    Some(&alloca.dest),
                    vec![num],
                    move |comp, args| {
                        let layout = layout.repeat(Packing::None, args[0].as_u64());
                        comp.process.alloc(comp.threadid, layout)
                    },
                ).await;
                self.allocs.push(thunk);
            }
            Instruction::Store(store) => {
                let address = self.get_temp(flow, &store.address).await;
                let value = self.get_temp(flow, &store.value).await;
                let deps = vec![address, value];
                self.add_thunk(flow, name, None, deps, move |comp, args| {
                    comp.process.store(comp.threadid, args[0], args[1], store.atomicity.as_ref());
                    Value::from(())
                }).await;
            }
            Instruction::Load(load) => {
                let deps = vec![self.get_temp(flow, &load.address).await];
                let layout = flow.ctx().layout(&flow.ctx().target_of(&flow.ctx().type_of(self.ectx, &load.address)));
                self.add_thunk(flow, name, Some(&load.dest), deps, move |comp, args| {
                    comp.process.load(comp.threadid, args[0], layout, load.atomicity.as_ref())
                }).await;
            }
            Instruction::Add(Add { dest, operand0, operand1, .. }) |
            Instruction::Sub(Sub { dest, operand0, operand1, .. }) |
            Instruction::Mul(Mul { dest, operand0, operand1, .. }) |
            Instruction::UDiv(UDiv { dest, operand0, operand1, .. }) |
            Instruction::SDiv(SDiv { dest, operand0, operand1, .. }) |
            Instruction::URem(URem { dest, operand0, operand1, .. }) |
            Instruction::SRem(SRem { dest, operand0, operand1, .. }) |
            Instruction::Xor(Xor { dest, operand0, operand1, .. }) |
            Instruction::And(And { dest, operand0, operand1, .. }) |
            Instruction::Or(Or { dest, operand0, operand1, .. }) |
            Instruction::Shl(Shl { dest, operand0, operand1, .. }) |
            Instruction::LShr(LShr { dest, operand0, operand1, .. }) |
            Instruction::AShr(AShr { dest, operand0, operand1, .. }) |
            Instruction::ICmp(ICmp { dest, operand0, operand1, .. })
            => {
                let deps = vec![self.get_temp(flow, operand0).await, self.get_temp(flow, operand1).await];
                let ctx = flow.ctx().clone();
                let ty1 = flow.ctx().type_of(self.ectx, operand0);
                let ty2 = flow.ctx().type_of(self.ectx, operand1);
                self.add_thunk(flow, name, Some(dest), deps, move |comp, args|
                    Self::binary(&*ctx, instr, &ty1, args[0], &ty2, args[1])).await;
            }
            Instruction::SExt(SExt { dest, operand, .. }) |
            Instruction::ZExt(ZExt { dest, operand, .. }) |
            Instruction::Trunc(Trunc { dest, operand, .. }) |
            Instruction::PtrToInt(PtrToInt { dest, operand, .. }) |
            Instruction::IntToPtr(IntToPtr { dest, operand, .. }) |
            Instruction::BitCast(BitCast { dest, operand, .. }) => {
                let deps = vec![self.get_temp(flow, operand).await];
                let ctx = flow.ctx().clone();
                let ty = flow.ctx().type_of(self.ectx, operand);
                self.add_thunk(flow, name, Some(dest), deps,
                               move |comp, args|
                                   Self::unary(&*ctx, instr, &ty, args[0])).await;
            }
            Instruction::GetElementPtr(gep) => {
                let mut deps = vec![self.get_temp(flow, &gep.address).await];
                for ind in gep.indices.iter() {
                    deps.push(self.get_temp(flow, ind).await);
                }
                let ectx = self.ectx;
                let ty = flow.ctx().type_of(self.ectx, &gep.address);
                let ctx = flow.ctx().clone();
                self.add_thunk(flow, name, Some(&gep.dest), deps, move |comp, args| {
                    let (_, offset) = ctx.offset_bit(&ty,
                                                     args[1..].iter().map(|v| v.as_i64()));
                    assert_eq!(offset % 8, 0);
                    ctx.value_from_address(add_u64_i64(args[0].as_u64(), offset / 8))
                }).await;
            }
            Instruction::InsertElement(InsertElement { vector, element, index, dest, .. }) => {
                let deps = vec![
                    self.get_temp(flow, vector).await,
                    self.get_temp(flow, element).await,
                    self.get_temp(flow, index).await];
                let ty = flow.ctx().type_of(self.ectx, vector);
                self.add_thunk(flow, name, Some(dest), deps, move |comp, args| {
                    let mut result = args[0].clone();
                    comp.ctx().insert_value(&ty, &mut result, args[1], iter::once(args[2].as_i64()));
                    result
                }).await;
            }
            Instruction::InsertValue(InsertValue { aggregate, element, dest, indices, .. }) => {
                let deps = vec![self.get_temp(flow, aggregate).await, self.get_temp(flow, element).await];
                let ty = flow.ctx().type_of(self.ectx, aggregate);
                //let (_, offset) = flow.ctx().offset_of(&ty, indices.iter().map(|i| *i as i64));
                self.add_thunk(flow, name, Some(dest), deps, move |comp, args| {
                    let mut aggregate = args[0].clone();
                    comp.ctx().insert_value(&ty, &mut aggregate, args[1], indices.iter().map(|x| *x as i64));
                    aggregate
                }).await;
            }
            Instruction::AtomicRMW(AtomicRMW { address, value, dest, operation, atomicity, .. }) => {
                let deps = vec![self.get_temp(flow, address).await, self.get_temp(flow, value).await];
                let ty = flow.ctx().target_of(&flow.ctx().type_of(self.ectx, address));
                let layout = flow.ctx().layout(&ty);

                self.add_thunk(flow, name, Some(dest), deps, move |comp, args| {
                    let current = comp.process.load(comp.threadid, args[0], layout, Some(atomicity)).clone();

                    let new = match operation {
                        RMWBinOp::Add => BinOp::Add,
                        RMWBinOp::Sub => BinOp::Sub,
                        RMWBinOp::Xchg => BinOp::Xchg,
                        RMWBinOp::And => BinOp::And,
                        RMWBinOp::Or => BinOp::Or,
                        RMWBinOp::Xor => BinOp::Xor,
                        _ => todo!("{:?}", operation),
                    }.call(comp.ctx(), &ty, &current, &ty, args[1]);
                    comp.process.store(comp.threadid, args[0], &new, Some(atomicity));
                    current
                }).await;
            }
            Instruction::Select(Select { condition, true_value, false_value, dest, .. }) => {
                let deps =
                    vec![self.get_temp(flow, condition).await,
                         self.get_temp(flow, true_value).await,
                         self.get_temp(flow, false_value).await];
                self.add_thunk(flow, name, Some(dest), deps, |comp, args|
                    if args[0].unwrap_bool() {
                        args[1].clone()
                    } else {
                        args[2].clone()
                    },
                ).await;
            }
            Instruction::ExtractValue(ExtractValue { aggregate, dest, indices, .. }) => {
                let deps = vec![self.get_temp(flow, aggregate).await];
                let ty = flow.ctx().type_of(self.ectx, aggregate);
                self.add_thunk(flow, name, Some(dest), deps, move |comp, args| {
                    comp.ctx().extract_value(&ty, args[0], indices.iter().map(|x| *x as i64))
                }).await;
            }
            Instruction::CmpXchg(CmpXchg { address, expected, replacement, dest, atomicity, .. }) => {
                let deps =
                    vec![self.get_temp(flow, address).await,
                         self.get_temp(flow, expected).await,
                         self.get_temp(flow, replacement).await];
                let ty = flow.ctx().target_of(&flow.ctx().type_of(self.ectx, address));
                let layout = flow.ctx().layout(&ty);
                self.add_thunk(flow, name, Some(dest), deps, move |comp, args| {
                    let (address, expected, replacement) = (args[0], args[1], args[2]);
                    let old = comp.process.load(comp.threadid, address, layout, Some(atomicity));
                    let success = &old == expected;
                    if success {
                        comp.process.store(comp.threadid, address, replacement, Some(atomicity));
                    }
                    Value::aggregate(vec![old.clone(), Value::from(success)].into_iter(), Packing::None)
                }).await;
            }
            Instruction::Fence(Fence { atomicity, .. }) => {
                //TODO
            }
            Instruction::ExtractElement(ExtractElement { vector, index, dest, .. }) => {
                let deps =
                    vec![self.get_temp(flow, vector).await,
                         self.get_temp(flow, index).await];
                let ectx = self.ectx;
                self.add_thunk(flow, name, Some(dest), deps, move |comp, args| {
                    let ty = comp.ctx().type_of(ectx, vector);
                    comp.ctx().extract_value(&ty, args[0], iter::once(args[1].as_i64()))
                }).await;
            }
            Instruction::ShuffleVector(ShuffleVector { operand0, operand1, dest, mask, .. }) => {
                let deps = vec![
                    self.get_temp(flow, operand0).await,
                    self.get_temp(flow, operand1).await];
                let (mask_type, mask) = flow.ctx().get_constant(self.ectx, mask);
                let mask_width = flow.ctx().count(&mask_type);
                let ty0 = flow.ctx().type_of(self.ectx, operand0);
                let width0 = flow.ctx().count(&*ty0);
                let ty1 = flow.ctx().type_of(self.ectx, operand0);
                let width1 = flow.ctx().count(&*ty1);
                let (ty, _) = flow.ctx().field_bit(&*ty0, 1);
                let out_ty = flow.ctx().vector_of(ty, mask_width);
                self.add_thunk(flow, name, Some(dest), deps, move |comp, args| {
                    let mut result = comp.ctx().aggregate_zero(&out_ty);
                    for i in 0..mask_width {
                        let index = comp.ctx().extract_value(&mask_type, &mask, iter::once(i as i64)).unwrap_u32() as i64;
                        let value = if index < width0 as i64 {
                            comp.ctx().extract_value(&ty0, args[0], iter::once(index))
                        } else {
                            comp.ctx().extract_value(&ty1, args[1], iter::once(index - width0 as i64))
                        };
                        comp.ctx().insert_value(&out_ty, &mut result, &value, iter::once(index));
                    }
                    result
                }).await;
            }
            _ => todo!("{:?}", instr),
        }
    }
    fn unary(ctx: &Ctx<'ctx>, instr: &Instruction, ty: &TypeRef, value: &Value) -> Value {
        match instr {
            Instruction::SExt(SExt { to_type, .. }) =>
                scast(ctx, ty, value, to_type),
            Instruction::ZExt(ZExt { to_type, .. }) |
            Instruction::Trunc(Trunc { to_type, .. }) |
            Instruction::PtrToInt(PtrToInt { to_type, .. }) |
            Instruction::IntToPtr(IntToPtr { to_type, .. }) |
            Instruction::BitCast(BitCast { to_type, .. }) =>
                ucast(ctx, ty, value, to_type),
            _ => unreachable!()
        }
    }
    fn binary(ctx: &Ctx<'ctx>, instr: &Instruction, ty1: &TypeRef, x: &Value, ty2: &TypeRef, y: &Value) -> Value {
        match instr {
            Instruction::Add(Add { .. }) => BinOp::Add,
            Instruction::Sub(Sub { .. }) => BinOp::Sub,
            Instruction::Mul(Mul { .. }) => BinOp::Mul,
            Instruction::UDiv(UDiv { .. }) => BinOp::UDiv,
            Instruction::SDiv(SDiv { .. }) => BinOp::SDiv,
            Instruction::URem(URem { .. }) => BinOp::URem,
            Instruction::SRem(SRem { .. }) => BinOp::SRem,
            Instruction::Xor(Xor { .. }) => BinOp::Xor,
            Instruction::And(And { .. }) => BinOp::And,
            Instruction::Or(Or { .. }) => BinOp::Or,
            Instruction::Shl(Shl { .. }) => BinOp::Shl,
            Instruction::LShr(LShr { .. }) => BinOp::LShr,
            Instruction::AShr(AShr { .. }) => BinOp::AShr,
            Instruction::ICmp(ICmp { predicate, .. }) => BinOp::ICmp(*predicate),
            _ => todo!(),
        }.call(ctx, ty1, x, ty2, y)
    }
    // fn icmp(predicate: &IntPredicate, x: &Value, y: &Value) -> bool {
    //     match predicate {
    //         IntPredicate::EQ => x.as_u128() == y.as_u128(),
    //         IntPredicate::NE => x.as_u128() != y.as_u128(),
    //         IntPredicate::ULE => x.as_u128() <= y.as_u128(),
    //         IntPredicate::ULT => x.as_u128() < y.as_u128(),
    //         IntPredicate::UGE => x.as_u128() >= y.as_u128(),
    //         IntPredicate::UGT => x.as_u128() > y.as_u128(),
    //         IntPredicate::SGT => x.as_i128() > y.as_i128(),
    //         IntPredicate::SGE => x.as_i128() >= y.as_i128(),
    //         IntPredicate::SLT => x.as_i128() < y.as_i128(),
    //         IntPredicate::SLE => x.as_i128() <= y.as_i128(),
    //     }
    // }
    async fn decode_term<'a>(&'a mut self, flow: &'a FlowCtx<'ctx, 'a>, term: &'ctx Terminator) -> DecodeResult<'ctx> {
        let name = format!("{} {:?}", term, term.get_debug_loc());
        match term {
            Terminator::Br(br) => {
                DecodeResult::Jump(&br.dest)
            }
            Terminator::Ret(ret) => {
                for x in self.allocs.iter() {
                    flow.data().thunk(name.clone(), flow.backtrace().clone(), vec![x.clone()], |comp, args| {
                        comp.process.free(comp.threadid, &args[0]);
                        Value::from(())
                    }).await;
                }
                let result = if let Some(oper) = ret.return_operand.as_ref() {
                    self.get_temp(flow, oper).await
                } else {
                    flow.data().constant(flow.backtrace().clone(), Value::from(())).await
                };
                DecodeResult::Return(result)
            }
            Terminator::CondBr(condbr) => {
                if self.get_temp(flow, &condbr.condition).await.await.unwrap_bool() {
                    DecodeResult::Jump(&condbr.true_dest)
                } else {
                    DecodeResult::Jump(&condbr.false_dest)
                }
            }
            Terminator::Unreachable(_) => {
                panic!("Unreachable");
            }
            Terminator::Invoke(invoke) => {
                self.decode_call(flow, &invoke.function, &invoke.arguments, Some(&invoke.result)).await;
                DecodeResult::Jump(&invoke.return_label)
            }
            Terminator::Switch(switch) => {
                let cond = self.get_temp(flow, &switch.operand).await;
                let cond = cond.await;
                let mut target = None;
                for (pat, dest) in switch.dests.iter() {
                    let (_, pat) = flow.ctx().get_constant(self.ectx, &pat);
                    if pat == cond {
                        target = Some(dest);
                    }
                }
                DecodeResult::Jump(target.unwrap_or(&switch.default_dest))
            }
            term => todo!("{:?}", term)
        }
    }
    async fn get_temp<'a>(&'a mut self, flow: &'a FlowCtx<'ctx, 'a>, oper: &'ctx Operand) -> Thunk<'ctx> {
        match oper {
            Operand::LocalOperand { name, .. } => {
                self.temps.get(name).unwrap_or_else(|| panic!("No variable named {:?} in \n{:#?}", name, self)).clone()
            }
            Operand::ConstantOperand(constant) => {
                let value = flow.ctx().get_constant(self.ectx, constant).1;
                self.add_thunk(flow, constant.to_string(), None, vec![], move |_, _| value).await
            }
            Operand::MetadataOperand => {
                self.add_thunk(flow, "metadata".to_string(), None, vec![], |_, _| Value::from(())).await
            }
        }
    }
    async fn decode_call<'a>(&'a mut self,
                             flow: &'a FlowCtx<'ctx, 'a>,
                             function: &'ctx Either<InlineAssembly, Operand>,
                             arguments: &'ctx Vec<(Operand, Vec<ParameterAttribute>)>,
                             dest: Option<&'ctx Name>,
    ) {
        let fun = match function {
            Either::Left(assembly) => todo!("{:?}", assembly),
            Either::Right(fun) => fun,
        };
        let fun_thunk = self.get_temp(flow, fun).await;
        let fun = fun_thunk.clone().await;
        let fun = flow.ctx().try_reverse_lookup(&fun).unwrap_or_else(|| panic!("Unknown function {:?} {:#?} {:#?}", fun, flow, fun_thunk.full_debug(&flow.ctx())));
        let fun = flow.ctx().functions.get(&fun).cloned().unwrap_or_else(|| panic!("Unknown function {:?}", fun));
        let mut deps = vec![];
        for (oper, _) in arguments.iter() {
            deps.push(self.get_temp(flow, oper).await)
        }
        let result = fun.call_imp(flow, &deps).await;
        if let Some(dest) = dest {
            self.temps.insert(dest, result);
        }
    }
    async fn add_thunk<'a>(&'a mut self,
                           flow: &'a FlowCtx<'ctx, 'a>,
                           name: String,
                           dest: Option<&'ctx Name>,
                           deps: Vec<Thunk<'ctx>>,
                           compute: impl 'ctx + FnOnce(ComputeCtx<'ctx, '_>, &[&Value]) -> Value) -> Thunk<'ctx> {
        let thunk = flow.data().thunk(name, flow.backtrace().clone(), deps, compute).await;
        if let Some(dest) = dest {
            self.temps.insert(dest, thunk.clone());
        }
        thunk
    }
}

impl<'ctx> Debug for InterpFrame<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let temps =
            self.temps.iter()
                .map(|(k, v)| (format!("{}", k), v))
                .collect::<HashMap<_, _>>();
        f.debug_struct("Frame")
            .field("fun", &self.fun.name)
            .field("temps", &temps)
            .finish()
    }
}

