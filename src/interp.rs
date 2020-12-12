use llvm_ir::{Function, BasicBlock, Name, Instruction, IntPredicate, Terminator, Operand, DebugLoc, Type, HasDebugLoc, TypeRef};
use std::collections::HashMap;
use crate::function::{Func};
use crate::data::{Thunk, ComputeCtx};
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
use crate::symbols::Symbol;
use crate::layout::{AggrLayout, Layout, Packing};
use crate::compile::{CFunc, CBlockId, CInstr, CLocal, COperand, CTerm, CCompute};
use vec_map::VecMap;
use itertools::Itertools;
use crate::operation::COperationName;

pub struct InterpFrame {
    origin: Option<CBlockId>,
    temps: VecMap<Thunk>,
    allocs: Vec<Thunk>,
}

struct InterpCtx {
    flow: FlowCtx,
}

enum DecodeResult {
    Jump(CBlockId),
    Return(Thunk),
}


impl InterpFrame {
    pub async fn call(flow: &FlowCtx, fun: &CFunc, params: Vec<Thunk>) -> Thunk {
        let mut temps = VecMap::with_capacity(fun.locals.len());
        for (arg, param) in fun.args.iter().zip_eq(params.into_iter()) {
            temps.insert(arg.0, param);
        }
        InterpFrame {
            origin: None,
            temps,
            allocs: vec![],
        }.decode(flow, fun).await
    }
    pub fn assign(&mut self, dest: CLocal, thunk: Thunk) {
        self.temps.insert(dest.0, thunk);
    }
    async fn decode_operand(&self, ctx: &InterpCtx, operand: &COperand) -> Thunk {
        match operand {
            COperand::Constant(class, value) =>
                ctx.flow.constant(value.clone()).await,
            COperand::Local(class, var) => {
                self.temps.get(var.0).unwrap_or_else(|| panic!("No local {:?}", var)).clone()
            }
            COperand::Inline => todo!(),
        }
    }
    async fn decode_operands(&self, ctx: &InterpCtx, operands: impl Iterator<Item=&COperand>) -> Vec<Thunk> {
        let mut result = Vec::with_capacity(operands.size_hint().0);
        for operand in operands {
            result.push(self.decode_operand(ctx, operand).await);
        }
        result
    }
    pub async fn decode(&mut self, flow: &FlowCtx, fun: &CFunc) -> Thunk {
        let mut block = &fun.blocks[0];
        loop {
            let mut phis = Vec::with_capacity(block.phis.len());
            for phi in block.phis.iter() {
                let origin = self.origin.unwrap();
                phis.push((phi.dest, self.decode_operand(&InterpCtx { flow: flow.clone() }, phi.mapping.get(&origin).unwrap()).await))
            }
            for (dest, thunk) in phis {
                self.assign(dest, thunk)
            }
            for instr in block.instrs.iter() {
                self.decode_instr(&InterpCtx {
                    flow: flow.with_frame(instr.frame.clone()),
                }, &instr.instr).await
            }
            match self.decode_term(&InterpCtx {
                flow: flow.with_frame(block.term.frame.clone()),
            }, &block.term.term).await {
                DecodeResult::Jump(name) => {
                    self.origin = Some(block.id);
                    block = &fun.blocks[name.0];
                }
                DecodeResult::Return(result) => return result,
            };
        }
    }
    // pub async fn call<'flow>(flow: &'flow FlowCtx<'ctx, 'flow>,
    //                          ectx: EvalCtx,
    //                          fp: u64,
    //                          fun: &'ctx Function,
    //                          params: Vec<Thunk<'ctx>>) -> Thunk<'ctx> {
    //     let temps =
    //         fun.parameters.iter()
    //             .zip(params.into_iter())
    //             .map(|(n, v)| (&n.name, v))
    //             .collect();
    //     let blocks =
    //         fun.basic_blocks.iter()
    //             .map(|block| (&block.name, block))
    //             .collect();
    //     let mut frame = InterpFrame {
    //         fp,
    //         fun,
    //         origin: None,
    //         temps,
    //         allocs: vec![],
    //         result: None,
    //         blocks,
    //         ectx,
    //     };
    //     frame.decode(flow).await
    // }
    // pub fn decode<'flow>(&'flow mut self, flow: &'flow FlowCtx<'ctx, 'flow>) -> impl Future<Output=Thunk<'ctx>> + 'flow {
    //     Box::pin(async move { self.decode_impl(flow).await })
    // }
    // pub async fn decode_impl<'flow>(&mut self, flow: &'flow FlowCtx<'ctx, 'flow>) -> Thunk<'ctx> {
    //     let mut block = self.fun.basic_blocks.get(0).unwrap();
    //     loop {
    //         let mut iter = block.instrs.iter().peekable();
    //         let mut phis = vec![];
    //         while let Some(Instruction::Phi(phi)) = iter.peek() {
    //             iter.next();
    //             let (oper, _) =
    //                 phi.incoming_values.iter()
    //                     .find(|(_, name)| Some(name) == self.origin)
    //                     .unwrap_or_else(|| panic!("No target {:?} in {:?}", self.origin, self.fun.name));
    //             let deps = vec![self.get_temp(flow, oper).await];
    //             phis.push((phi, deps));
    //         }
    //         for (phi, deps) in phis {
    //             self.add_thunk(flow, format!("{}", phi), Some(&phi.dest), deps, |comp, args| args[0].clone()).await;
    //         }
    //         for instr in iter {
    //             InterpCtx {
    //                 ctx: flow.ctx(),
    //                 flow: flow.with_frame(BacktraceFrame { ip: self.fp, ..BacktraceFrame::default() }),
    //                 frame: self,
    //                 name: format!("{} {:?}", instr, instr.get_debug_loc()),
    //             }.decode_instr(instr).await;
    //         }
    //         match (InterpCtx {
    //             ctx: flow.ctx(),
    //             flow: flow.with_frame(BacktraceFrame { ip: self.fp, ..BacktraceFrame::default() }),
    //             frame: self,
    //             name: format!("{} {:?}", block.term, block.term.get_debug_loc()),
    //         }.decode_term(&block.term).await) {
    //             DecodeResult::Jump(name) => {
    //                 self.origin = Some(&block.name);
    //                 block = *self.blocks.get(name).unwrap();
    //             }
    //             DecodeResult::Return(result) => return result,
    //         };
    //     }
    // }
    // async fn add_thunk<'a>(&'a mut self,
    //                        flow: &'a FlowCtx<'ctx, 'a>,
    //                        name: String,
    //                        dest: Option<&'ctx Name>,
    //                        deps: Vec<Thunk<'ctx>>,
    //                        compute: impl 'ctx + FnOnce(ComputeCtx<'ctx, '_>, &[&Value]) -> Value) -> Thunk<'ctx> {
    //     let thunk = flow.data().thunk(name, flow.backtrace().clone(), deps, compute).await;
    //     if let Some(dest) = dest {
    //         self.temps.insert(dest, thunk.clone());
    //     }
    //     thunk
    // }
    // async fn get_temp<'flow>(&'flow mut self, flow: &FlowCtx<'ctx, 'flow>, oper: &'ctx Operand) -> Thunk<'ctx> {
    //     match oper {
    //         Operand::LocalOperand { name, .. } => {
    //             self.temps.get(name).unwrap_or_else(|| panic!("No variable named {:?} in \n{:#?}", name, self)).clone()
    //         }
    //         Operand::ConstantOperand(constant) => {
    //             let value = flow.ctx().get_constant(self.ectx, constant).1;
    //             self.add_thunk(flow, constant.to_string(), None, vec![], move |_, _| value).await
    //         }
    //         Operand::MetadataOperand => {
    //             self.add_thunk(flow, "metadata".to_string(), None, vec![], |_, _| Value::from(())).await
    //         }
    //     }
    // }
    async fn decode_instr(&mut self, ctx: &InterpCtx, instr: &CInstr) {
        match instr {
            CInstr::Compute(compute) => {
                let deps = self.decode_operands(ctx, compute.operands.iter()).await;
                let result =
                    self.thunk(ctx, deps, {
                        let compute = compute.clone();
                        move |cc, args| {
                            compute.operation.call_operation(args)
                        }
                    }).await;
                self.assign(compute.dest, result);
            }
            CInstr::Call(call) => {
                let f = self.decode_operand(ctx, &call.func).await;
                let args = self.decode_operands(ctx, call.arguments.iter()).await;
                let imp = ctx.flow.process().reverse_lookup_fun(&f.await);
                let result = imp.call_imp(&ctx.flow, &args).await;
                if let Some(dest) = call.dest {
                    self.assign(dest, result)
                }
            }
            CInstr::Alloca(alloca) => {
                let result = self.thunk(
                    ctx,
                    vec![self.decode_operand(ctx, &alloca.count).await],
                    {
                        let alloca = alloca.clone();
                        move |comp, args| {
                            comp.process.alloc(comp.threadid,
                                               alloca.layout.repeat(Packing::None, args[0].as_u64()))
                        }
                    }).await;
                self.assign(alloca.dest, result);
            }
            CInstr::Load(load) => {
                let address = self.decode_operand(ctx, &load.address).await;
                let result =
                    ctx.flow.data().load(
                        ctx.flow.backtrace().clone(),
                        address,
                        load.address.class().target().layout(),
                        load.atomicity.clone()).await;
                self.assign(load.dest, result);
            }
            CInstr::Store(store) => {
                let address = self.decode_operand(ctx, &store.address).await;
                let value = self.decode_operand(ctx, &store.value).await;
                ctx.flow.data().store(
                    ctx.flow.backtrace().clone(),
                    address,
                    value,
                    store.atomicity.clone()).await;
            }
            CInstr::CmpXchg(cmpxchg) => {
                let address = self.decode_operand(ctx, &cmpxchg.address).await;
                let expected = self.decode_operand(ctx, &cmpxchg.expected).await;
                let replacement = self.decode_operand(ctx, &cmpxchg.replacement).await;
                let deps = vec![address, expected, replacement];
                let layout = cmpxchg.expected.class().layout();
                let types = ctx.flow.process().types();
                let class = types.struc(vec![cmpxchg.expected.class().clone(), types.bool()], false);
                let result = self.thunk(ctx, deps, {
                    let cmpxchg = cmpxchg.clone();
                    move |comp, args| {
                        let current = comp.process.load(comp.threadid, args[0], layout, None).clone();
                        let exchanged = &current == args[1];
                        if exchanged {
                            comp.process.store(comp.threadid, args[0], args[2], None);
                        }
                        Value::aggregate(&class, vec![current, Value::from(exchanged)].into_iter())
                    }
                }).await;
                self.assign(cmpxchg.dest, result);
            }
            CInstr::AtomicRMW(atomicrmw) => {
                let address = self.decode_operand(ctx, &atomicrmw.address).await;
                let value = self.decode_operand(ctx, &atomicrmw.value).await;
                let deps = vec![address, value];
                let layout = atomicrmw.value.class().layout();
                let result = self.thunk(ctx, deps, {
                    let atomicrmw = atomicrmw.clone();
                    move |comp, args| {
                        let current = comp.process.load(comp.threadid, args[0], layout, Some(atomicrmw.atomicity.clone())).clone();
                        let new = atomicrmw.operation.call_operation(&[&current, args[1]]);
                        comp.process.store(comp.threadid, args[0], &new, Some(atomicrmw.atomicity.clone()));
                        current
                    }
                }).await;
                self.assign(atomicrmw.dest, result);
            }
            CInstr::Freeze => todo!(),
            CInstr::Fence => {
                //TODO
            }
            CInstr::Unknown(instr) => todo!("{:?}", instr),
        }
    }
    async fn decode_term(&mut self, ctx: &InterpCtx, term: &CTerm) -> DecodeResult {
        match term {
            CTerm::Ret(ret) => {
                DecodeResult::Return(self.decode_operand(ctx, &ret.return_operand).await)
            }
            CTerm::Br(br) => {
                DecodeResult::Jump(br.target)
            }
            CTerm::CondBr(condbr) => {
                if self.decode_operand(ctx, &condbr.condition).await.await.unwrap_bool() {
                    DecodeResult::Jump(condbr.true_target)
                } else {
                    DecodeResult::Jump(condbr.false_target)
                }
            }
            CTerm::Switch(switch) => {
                let condition = self.decode_operand(ctx, &switch.condition).await.await;
                DecodeResult::Jump(switch.targets.get(&condition).cloned().unwrap_or(switch.default_target))
            }
            CTerm::Invoke(invoke) => {
                let f = self.decode_operand(ctx, &invoke.function).await;
                let args = self.decode_operands(ctx, invoke.arguments.iter()).await;
                let imp = ctx.flow.process().reverse_lookup_fun(&f.await);
                let result = imp.call_imp(&ctx.flow, &args).await;
                self.assign(invoke.dest, result);
                DecodeResult::Jump(invoke.target)
            }
            CTerm::Unreachable(_) => todo!(),
            CTerm::Resume => todo!(),
        }
    }
    async fn thunk(
        &mut self,
        ctx: &InterpCtx,
        deps: Vec<Thunk>,
        compute: impl FnOnce(&ComputeCtx, &[&Value]) -> Value + 'static) -> Thunk {
        ctx.flow.data().thunk(ctx.flow.backtrace().clone(),
                              deps, compute).await
    }
    // async fn decode_instr<'a>(&'a mut self, instr: &'ctx Instruction) {
    //     let name = format!("{} {:?}", instr, instr.get_debug_loc());
    //     match instr {
    //         Instruction::Call(call) => {
    //             self.decode_call(&call.function, &call.arguments, call.dest.as_ref()).await;
    //         }
    //         Instruction::Alloca(alloca) => {
    //             let num = self.get_temp(&alloca.num_elements).await;
    //             let layout = self.ctx.layout(&alloca.allocated_type);
    //             let thunk = self.add_thunk(
    //                 Some(&alloca.dest),
    //                 vec![num],
    //                 move |comp, args| {
    //                     let layout = layout.repeat(Packing::None, args[0].as_u64());
    //                     comp.process.alloc(comp.threadid, layout)
    //                 },
    //             ).await;
    //             self.frame.allocs.push(thunk);
    //         }
    //         Instruction::Store(store) => {
    //             let address = self.get_temp(&store.address).await;
    //             let value = self.get_temp(&store.value).await;
    //             let deps = vec![address, value];
    //             self.add_thunk(None, deps, move |comp, args| {
    //                 comp.process.store(comp.threadid, args[0], args[1], store.atomicity.as_ref());
    //                 Value::from(())
    //             }).await;
    //         }
    //         Instruction::Load(load) => {
    //             let deps = vec![self.get_temp(&load.address).await];
    //             let layout = self.ctx.layout(&self.ctx.target_of(&self.ctx.type_of(self.frame.ectx, &load.address)));
    //             self.add_thunk(Some(&load.dest), deps, move |comp, args| {
    //                 comp.process.load(comp.threadid, args[0], layout, load.atomicity.as_ref())
    //             }).await;
    //         }
    //         Instruction::Add(Add { dest, operand0, operand1, .. }) |
    //         Instruction::Sub(Sub { dest, operand0, operand1, .. }) |
    //         Instruction::Mul(Mul { dest, operand0, operand1, .. }) |
    //         Instruction::UDiv(UDiv { dest, operand0, operand1, .. }) |
    //         Instruction::SDiv(SDiv { dest, operand0, operand1, .. }) |
    //         Instruction::URem(URem { dest, operand0, operand1, .. }) |
    //         Instruction::SRem(SRem { dest, operand0, operand1, .. }) |
    //         Instruction::Xor(Xor { dest, operand0, operand1, .. }) |
    //         Instruction::And(And { dest, operand0, operand1, .. }) |
    //         Instruction::Or(Or { dest, operand0, operand1, .. }) |
    //         Instruction::Shl(Shl { dest, operand0, operand1, .. }) |
    //         Instruction::LShr(LShr { dest, operand0, operand1, .. }) |
    //         Instruction::AShr(AShr { dest, operand0, operand1, .. }) |
    //         Instruction::ICmp(ICmp { dest, operand0, operand1, .. })
    //         => {
    //             let deps = vec![self.get_temp(operand0).await, self.get_temp(operand1).await];
    //             let ctx = self.ctx.clone();
    //             let ty1 = self.ctx.type_of(self.frame.ectx, operand0);
    //             let ty2 = self.ctx.type_of(self.frame.ectx, operand1);
    //             let ctx = self.ctx.clone();
    //             self.add_thunk(Some(dest), deps, move |comp, args|
    //                 Self::binary(&*ctx, instr, &ty1, args[0], &ty2, args[1])).await;
    //         }
    //         Instruction::SExt(SExt { dest, operand, .. }) |
    //         Instruction::ZExt(ZExt { dest, operand, .. }) |
    //         Instruction::Trunc(Trunc { dest, operand, .. }) |
    //         Instruction::PtrToInt(PtrToInt { dest, operand, .. }) |
    //         Instruction::IntToPtr(IntToPtr { dest, operand, .. }) |
    //         Instruction::BitCast(BitCast { dest, operand, .. }) => {
    //             let deps = vec![self.get_temp(operand).await];
    //             let ctx = self.ctx.clone();
    //             let ty = self.ctx.type_of(self.frame.ectx, operand);
    //             self.add_thunk(Some(dest), deps,
    //                            move |comp, args|
    //                                Self::unary(&*ctx, instr, &ty, args[0])).await;
    //         }
    //         Instruction::GetElementPtr(gep) => {
    //             let mut deps = vec![self.get_temp(&gep.address).await];
    //             for ind in gep.indices.iter() {
    //                 deps.push(self.get_temp(ind).await);
    //             }
    //             let ectx = self.frame.ectx;
    //             let ty = self.ctx.type_of(self.frame.ectx, &gep.address);
    //             let ctx = self.ctx.clone();
    //             self.add_thunk(Some(&gep.dest), deps, move |comp, args| {
    //                 let (_, offset) = ctx.offset_bit(&ty,
    //                                                  args[1..].iter().map(|v| v.as_i64()));
    //                 assert_eq!(offset % 8, 0);
    //                 ctx.value_from_address(add_u64_i64(args[0].as_u64(), offset / 8))
    //             }).await;
    //         }
    //         Instruction::InsertElement(InsertElement { vector, element, index, dest, .. }) => {
    //             let deps = vec![
    //                 self.get_temp(vector).await,
    //                 self.get_temp(element).await,
    //                 self.get_temp(index).await];
    //             let ty = self.ctx.type_of(self.frame.ectx, vector);
    //             self.add_thunk(Some(dest), deps, move |comp, args| {
    //                 let mut result = args[0].clone();
    //                 comp.ctx().insert_value(&ty, &mut result, args[1], iter::once(args[2].as_i64()));
    //                 result
    //             }).await;
    //         }
    //         Instruction::InsertValue(InsertValue { aggregate, element, dest, indices, .. }) => {
    //             let deps = vec![self.get_temp(aggregate).await, self.get_temp(element).await];
    //             let ty = self.ctx.type_of(self.frame.ectx, aggregate);
    //             //let (_, offset) = self.ctx.offset_of(&ty, indices.iter().map(|i| *i as i64));
    //             self.add_thunk(Some(dest), deps, move |comp, args| {
    //                 let mut aggregate = args[0].clone();
    //                 comp.ctx().insert_value(&ty, &mut aggregate, args[1], indices.iter().map(|x| *x as i64));
    //                 aggregate
    //             }).await;
    //         }
    //         Instruction::AtomicRMW(AtomicRMW { address, value, dest, operation, atomicity, .. }) => {
    //             let deps = vec![self.get_temp(address).await, self.get_temp(value).await];
    //             let ty = self.ctx.target_of(&self.ctx.type_of(self.frame.ectx, address));
    //             let layout = self.ctx.layout(&ty);
    //
    //             self.add_thunk(Some(dest), deps, move |comp, args| {
    //                 let current = comp.process.load(comp.threadid, args[0], layout, Some(atomicity)).clone();
    //
    //                 let new = match operation {
    //                     RMWBinOp::Add => BinOp::Add,
    //                     RMWBinOp::Sub => BinOp::Sub,
    //                     RMWBinOp::Xchg => BinOp::Xchg,
    //                     RMWBinOp::And => BinOp::And,
    //                     RMWBinOp::Or => BinOp::Or,
    //                     RMWBinOp::Xor => BinOp::Xor,
    //                     _ => todo!("{:?}", operation),
    //                 }.call(comp.ctx(), &ty, &current, &ty, args[1]);
    //                 comp.process.store(comp.threadid, args[0], &new, Some(atomicity));
    //                 current
    //             }).await;
    //         }
    //         Instruction::Select(Select { condition, true_value, false_value, dest, .. }) => {
    //             let deps =
    //                 vec![self.get_temp(condition).await,
    //                      self.get_temp(true_value).await,
    //                      self.get_temp(false_value).await];
    //             self.add_thunk(Some(dest), deps, |comp, args|
    //                 if args[0].unwrap_bool() {
    //                     args[1].clone()
    //                 } else {
    //                     args[2].clone()
    //                 },
    //             ).await;
    //         }
    //         Instruction::ExtractValue(ExtractValue { aggregate, dest, indices, .. }) => {
    //             let deps = vec![self.get_temp(aggregate).await];
    //             let ty = self.ctx.type_of(self.frame.ectx, aggregate);
    //             self.add_thunk(Some(dest), deps, move |comp, args| {
    //                 comp.ctx().extract_value(&ty, args[0], indices.iter().map(|x| *x as i64))
    //             }).await;
    //         }
    //         Instruction::CmpXchg(CmpXchg { address, expected, replacement, dest, atomicity, .. }) => {
    //             let deps =
    //                 vec![self.get_temp(address).await,
    //                      self.get_temp(expected).await,
    //                      self.get_temp(replacement).await];
    //             let ty = self.ctx.target_of(&self.ctx.type_of(self.frame.ectx, address));
    //             let layout = self.ctx.layout(&ty);
    //             self.add_thunk(Some(dest), deps, move |comp, args| {
    //                 let (address, expected, replacement) = (args[0], args[1], args[2]);
    //                 let old = comp.process.load(comp.threadid, address, layout, Some(atomicity));
    //                 let success = &old == expected;
    //                 if success {
    //                     comp.process.store(comp.threadid, address, replacement, Some(atomicity));
    //                 }
    //                 Value::aggregate(vec![old.clone(), Value::from(success)].into_iter(), Packing::None)
    //             }).await;
    //         }
    //         Instruction::Fence(Fence { atomicity, .. }) => {
    //             //TODO
    //         }
    //         Instruction::ExtractElement(ExtractElement { vector, index, dest, .. }) => {
    //             let deps =
    //                 vec![self.get_temp(vector).await,
    //                      self.get_temp(index).await];
    //             let ectx = self.frame.ectx;
    //             self.add_thunk(Some(dest), deps, move |comp, args| {
    //                 let ty = comp.ctx().type_of(ectx, vector);
    //                 comp.ctx().extract_value(&ty, args[0], iter::once(args[1].as_i64()))
    //             }).await;
    //         }
    //         Instruction::ShuffleVector(ShuffleVector { operand0, operand1, dest, mask, .. }) => {
    //             let deps = vec![
    //                 self.get_temp(operand0).await,
    //                 self.get_temp(operand1).await];
    //             let (mask_type, mask) = self.ctx.get_constant(self.frame.ectx, mask);
    //             let mask_width = self.ctx.count(&mask_type);
    //             let ty0 = self.ctx.type_of(self.frame.ectx, operand0);
    //             let width0 = self.ctx.count(&*ty0);
    //             let ty1 = self.ctx.type_of(self.frame.ectx, operand0);
    //             let width1 = self.ctx.count(&*ty1);
    //             let (ty, _) = self.ctx.field_bit(&*ty0, 1);
    //             let out_ty = self.ctx.vector_of(ty, mask_width);
    //             self.add_thunk(Some(dest), deps, move |comp, args| {
    //                 let mut result = comp.ctx().aggregate_zero(&out_ty);
    //                 for i in 0..mask_width {
    //                     let index = comp.ctx().extract_value(&mask_type, &mask, iter::once(i as i64)).unwrap_u32() as i64;
    //                     let value = if index < width0 as i64 {
    //                         comp.ctx().extract_value(&ty0, args[0], iter::once(index))
    //                     } else {
    //                         comp.ctx().extract_value(&ty1, args[1], iter::once(index - width0 as i64))
    //                     };
    //                     comp.ctx().insert_value(&out_ty, &mut result, &value, iter::once(index));
    //                 }
    //                 result
    //             }).await;
    //         }
    //         _ => todo!("{:?}", instr),
    //     }
    // }
    // fn unary(ctx: &Ctx<'ctx>, instr: &Instruction, ty: &TypeRef, value: &Value) -> Value {
    //     match instr {
    //         Instruction::SExt(SExt { to_type, .. }) =>
    //             scast(ctx, ty, value, to_type),
    //         Instruction::ZExt(ZExt { to_type, .. }) |
    //         Instruction::Trunc(Trunc { to_type, .. }) |
    //         Instruction::PtrToInt(PtrToInt { to_type, .. }) |
    //         Instruction::IntToPtr(IntToPtr { to_type, .. }) |
    //         Instruction::BitCast(BitCast { to_type, .. }) =>
    //             ucast(ctx, ty, value, to_type),
    //         _ => unreachable!()
    //     }
    // }
    // fn binary(ctx: &Ctx<'ctx>, instr: &Instruction, ty1: &TypeRef, x: &Value, ty2: &TypeRef, y: &Value) -> Value {
    //     match instr {
    //         Instruction::Add(Add { .. }) => BinOp::Add,
    //         Instruction::Sub(Sub { .. }) => BinOp::Sub,
    //         Instruction::Mul(Mul { .. }) => BinOp::Mul,
    //         Instruction::UDiv(UDiv { .. }) => BinOp::UDiv,
    //         Instruction::SDiv(SDiv { .. }) => BinOp::SDiv,
    //         Instruction::URem(URem { .. }) => BinOp::URem,
    //         Instruction::SRem(SRem { .. }) => BinOp::SRem,
    //         Instruction::Xor(Xor { .. }) => BinOp::Xor,
    //         Instruction::And(And { .. }) => BinOp::And,
    //         Instruction::Or(Or { .. }) => BinOp::Or,
    //         Instruction::Shl(Shl { .. }) => BinOp::Shl,
    //         Instruction::LShr(LShr { .. }) => BinOp::LShr,
    //         Instruction::AShr(AShr { .. }) => BinOp::AShr,
    //         Instruction::ICmp(ICmp { predicate, .. }) => BinOp::ICmp(*predicate),
    //         _ => todo!(),
    //     }.call(ctx, ty1, x, ty2, y)
    // }
    // async fn decode_term<'a>(&'a mut self, term: &'ctx Terminator) -> DecodeResult<'ctx> {
    //     match term {
    //         Terminator::Br(br) => {
    //             DecodeResult::Jump(&br.dest)
    //         }
    //         Terminator::Ret(ret) => {
    //             for x in self.frame.allocs.iter() {
    //                 self.flow.data().thunk(self.name.clone(), self.flow.backtrace().clone(), vec![x.clone()], |comp, args| {
    //                     comp.process.free(comp.threadid, &args[0]);
    //                     Value::from(())
    //                 }).await;
    //             }
    //             let result = if let Some(oper) = ret.return_operand.as_ref() {
    //                 self.get_temp(oper).await
    //             } else {
    //                 self.flow.data().constant(self.flow.backtrace().clone(), Value::from(())).await
    //             };
    //             DecodeResult::Return(result)
    //         }
    //         Terminator::CondBr(condbr) => {
    //             if self.get_temp(&condbr.condition).await.await.unwrap_bool() {
    //                 DecodeResult::Jump(&condbr.true_dest)
    //             } else {
    //                 DecodeResult::Jump(&condbr.false_dest)
    //             }
    //         }
    //         Terminator::Unreachable(_) => {
    //             panic!("Unreachable");
    //         }
    //         Terminator::Invoke(invoke) => {
    //             self.decode_call(&invoke.function, &invoke.arguments, Some(&invoke.result)).await;
    //             DecodeResult::Jump(&invoke.return_label)
    //         }
    //         Terminator::Switch(switch) => {
    //             let cond = self.get_temp(&switch.operand).await;
    //             let cond = cond.await;
    //             let mut target = None;
    //             for (pat, dest) in switch.dests.iter() {
    //                 let (_, pat) = self.ctx.get_constant(self.frame.ectx, &pat);
    //                 if pat == cond {
    //                     target = Some(dest);
    //                 }
    //             }
    //             DecodeResult::Jump(target.unwrap_or(&switch.default_dest))
    //         }
    //         term => todo!("{:?}", term)
    //     }
    // }
    // async fn decode_call<'a>(&'a mut self,
    //                          function: &'ctx Either<InlineAssembly, Operand>,
    //                          arguments: &'ctx Vec<(Operand, Vec<ParameterAttribute>)>,
    //                          dest: Option<&'ctx Name>,
    // ) {
    //     let fun = match function {
    //         Either::Left(assembly) => todo!("{:?}", assembly),
    //         Either::Right(fun) => fun,
    //     };
    //     let fun_thunk = self.get_temp(fun).await;
    //     let fun = fun_thunk.clone().await;
    //     let fun = self.ctx.try_reverse_lookup(&fun).unwrap_or_else(|| panic!("Unknown function {:?} {:#?} {:#?}", fun, self.flow, fun_thunk.full_debug(&self.ctx)));
    //     let fun = self.ctx.functions.get(&fun).cloned().unwrap_or_else(|| panic!("Unknown function {:?}", fun));
    //     let mut deps = vec![];
    //     for (oper, _) in arguments.iter() {
    //         deps.push(self.get_temp(oper).await)
    //     }
    //     let result = fun.call_imp(&self.flow, &deps).await;
    //     if let Some(dest) = dest {
    //         self.frame.temps.insert(dest, result);
    //     }
    // }
    // async fn get_temp<'flow>(&'flow mut self, oper: &'ctx Operand) -> Thunk<'ctx> {
    //     self.frame.get_temp(&self.flow, oper).await
    // }
    // async fn add_thunk<'a>(&'a mut self,
    //                        dest: Option<&'ctx Name>,
    //                        deps: Vec<Thunk<'ctx>>,
    //                        compute: impl 'ctx + FnOnce(ComputeCtx<'ctx, '_>, &[&Value]) -> Value) -> Thunk<'ctx> {
    //     self.frame.add_thunk(&self.flow, self.name.clone(), dest, deps, compute).await
    // }
}


