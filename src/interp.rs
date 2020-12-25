use std::{fmt, iter};
use std::collections::HashMap;
use std::fmt::Debug;
use std::rc::Rc;

use either::Either;
use itertools::Itertools;
use llvm_ir::{BasicBlock, DebugLoc, Function, HasDebugLoc, Instruction, IntPredicate, Name, Operand, Terminator, Type, TypeRef};
use llvm_ir::function::ParameterAttribute;
use llvm_ir::instruction::{Add, And, AShr, AtomicRMW, BitCast, CmpXchg, ExtractElement, ExtractValue, Fence, ICmp, InlineAssembly, InsertElement, InsertValue, IntToPtr, LShr, Mul, Or, PtrToInt, RMWBinOp, SDiv, Select, SExt, Shl, ShuffleVector, SRem, Sub, Trunc, UDiv, URem, Xor, ZExt};
use smallvec::SmallVec;
use smallvec::smallvec;
use vec_map::VecMap;

use crate::async_timer;
use crate::backtrace::{Backtrace, BacktraceFrame};
use crate::compile::function::{CBlockId, CFunc, CInstr, CLocal, COperand, CTerm};
use crate::data::{ComputeCtx, Thunk, ThunkDeps};
use crate::flow::FlowCtx;
use crate::function::{Func, Panic};
use crate::layout::{AggrLayout, Layout, Packing};
use crate::ordering::Ordering;
use crate::symbols::Symbol;
use crate::timer;
use crate::value::{add_u64_i64, Value};

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
    pub async fn call(flow: &FlowCtx, fun: &CFunc, params: Vec<Thunk>) -> Result<Thunk, Panic> {
        flow.recursor().spawn(Self::call_impl(flow, fun, params)).await
        //Self::call_impl(flow, fun, params).await
    }
    pub async fn call_impl(flow: &FlowCtx, fun: &CFunc, params: Vec<Thunk>) -> Result<Thunk, Panic> {
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
        let inner = async {
            match operand {
                COperand::Constant(class, value) => value.clone(),
                COperand::Local(class, var) => {
                    self.temps.get(var.0).unwrap_or_else(|| panic!("No local {:?}", var)).clone()
                }
                COperand::Inline => todo!(),
                COperand::Expr(expr) => ctx.flow.constant(ctx.flow.evaluate(expr)).await,
            }
        };
        //async_timer!("InterpFrame::decode_operand", inner).await
        inner.await
    }
    async fn decode_operands(&self, ctx: &InterpCtx, operands: impl Iterator<Item=&COperand>) -> ThunkDeps {
        let inner = async {
            let mut result = SmallVec::with_capacity(operands.size_hint().0);
            for operand in operands {
                result.push(self.decode_operand(ctx, operand).await);
            }
            result
        };
        //async_timer!("InterpFrame::decode_operands", inner).await
        inner.await
    }
    pub async fn decode(&mut self, flow: &FlowCtx, fun: &CFunc) -> Result<Thunk, Panic> {
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
                }, &instr.instr).await?
            }
            match self.decode_term(&InterpCtx {
                flow: flow.with_frame(block.term.frame.clone()),
            }, &block.term.term).await? {
                DecodeResult::Jump(name) => {
                    self.origin = Some(block.id);
                    block = &fun.blocks[name.0];
                }
                DecodeResult::Return(result) => return Ok(result),
            };
        }
    }
    async fn decode_instr(&mut self, ctx: &InterpCtx, instr: &CInstr) -> Result<(), Panic> {
        if ctx.flow.data().tracing() {
            //println!("{} {} {:?}", ctx.flow.data().threadid().0, ctx.flow.data().seq(), instr);
        }
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
                let imp = ctx.flow.process().definitions.reverse_lookup_fun(f.await.as_u64()).unwrap();
                let result = imp.call_imp(&ctx.flow, &args).await?;
                if let Some(dest) = call.dest {
                    self.assign(dest, result)
                }
            }
            CInstr::Alloca(alloca) => {
                let result = self.thunk(
                    ctx,
                    smallvec![self.decode_operand(ctx, &alloca.count).await],
                    {
                        let alloca = alloca.clone();
                        move |comp, args| {
                            comp.process.addr(comp.process.memory.alloc(
                                comp.threadid,
                                alloca.layout.repeat(Packing::None, args[0].as_u64())))
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
                        load.atomicity).await;
                self.assign(load.dest, result);
            }
            CInstr::Store(store) => {
                let address = self.decode_operand(ctx, &store.address).await;
                let value = self.decode_operand(ctx, &store.value).await;
                let layout = store.value.class().layout();
                ctx.flow.data().store(
                    ctx.flow.backtrace().clone(),
                    address,
                    layout,
                    value,
                    store.atomicity).await;
            }
            CInstr::CmpXchg(cmpxchg) => {
                let address = self.decode_operand(ctx, &cmpxchg.address).await;
                let expected = self.decode_operand(ctx, &cmpxchg.expected).await;
                let replacement = self.decode_operand(ctx, &cmpxchg.replacement).await;
                let class = cmpxchg.expected.class();
                let result =
                    ctx.flow.cmpxchg(class, address, expected, replacement, Ordering::from(cmpxchg.success), Ordering::from(cmpxchg.failure)).await;
                self.assign(cmpxchg.dest, result);
            }
            CInstr::AtomicRMW(atomicrmw) => {
                let address = self.decode_operand(ctx, &atomicrmw.address).await;
                let value = self.decode_operand(ctx, &atomicrmw.value).await;
                let atomicrmw = atomicrmw.clone();
                let dest = atomicrmw.dest;
                let result = ctx.flow.atomicrmw(
                    atomicrmw.value.class().clone(),
                    address,
                    value,
                    Ordering::from(atomicrmw.atomicity),
                    move |current, operand| {
                        atomicrmw.operation.call_operation(&[&current, &operand])
                    }).await;
                self.assign(dest, result);
            }
            CInstr::Freeze => todo!(),
            CInstr::Fence => {
                //TODO
            }
            CInstr::Unknown(instr) => todo!("{:?}", instr),
        }
        Ok(())
    }
    async fn decode_term(&mut self, ctx: &InterpCtx, term: &CTerm) -> Result<DecodeResult, Panic> {
        if ctx.flow.data().tracing() {
            //println!("{} {} {:?}", ctx.flow.data().threadid().0, ctx.flow.data().seq(), term);
        }
        Ok(match term {
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
                let imp =
                    ctx.flow.process()
                        .definitions
                        .reverse_lookup_fun(f.clone().await.as_u64())
                        .unwrap_or_else(|e| panic!("{} {:#?}", e, f.full_debug()));
                let result = imp.call_imp(&ctx.flow, &args).await?;
                self.assign(invoke.dest, result);
                DecodeResult::Jump(invoke.target)
            }
            CTerm::Unreachable(_) => panic!(),
            CTerm::Resume => todo!(),
        })
    }
    async fn thunk(
        &mut self,
        ctx: &InterpCtx,
        deps: ThunkDeps,
        compute: impl FnOnce(&ComputeCtx, &[&Value]) -> Value + 'static) -> Thunk {
        ctx.flow.data().thunk(ctx.flow.backtrace().clone(),
                              deps, compute).await
    }
}


