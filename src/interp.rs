use llvm_ir::{Function, BasicBlock, Name, Instruction, IntPredicate, Terminator, Operand};
use std::collections::HashMap;
use crate::ctx::{EvalCtx, Ctx};
use crate::function::{Func, ExecArgs};
use crate::data::{DataFlow, Thunk, ComputeArgs};
use std::rc::Rc;
use futures::future::LocalBoxFuture;
use llvm_ir::instruction::{RMWBinOp, Sub, Mul, UDiv, SDiv, URem, SRem, Add, And, Or, Shl, LShr, AShr, ICmp, Xor, SExt, ZExt, Trunc, PtrToInt, IntToPtr, BitCast, InsertValue, AtomicRMW, Select, ExtractValue, CmpXchg, Fence, InlineAssembly};
use either::Either;
use std::fmt;
use futures::{Future, FutureExt};
use crate::value::{Value, add_u64_i64};
use llvm_ir::function::ParameterAttribute;
use std::fmt::Debug;
use crate::backtrace::{Backtrace, BacktraceFrame};

#[derive(Debug)]
pub struct InterpFunc<'ctx> {
    pub src: &'ctx Function,
    pub ectx: EvalCtx,
}

impl<'ctx> InterpFunc<'ctx> {
    pub fn new(ectx: EvalCtx, fun: &'ctx Function) -> Self {
        InterpFunc {
            src: &fun,
            ectx: ectx,
        }
    }
}


struct InterpFrame<'ctx> {
    pub ctx: Rc<Ctx<'ctx>>,
    pub fun: &'ctx Function,
    pub origin: Option<&'ctx Name>,
    pub temps: HashMap<&'ctx Name, Rc<Thunk<'ctx>>>,
    pub allocs: Vec<Rc<Thunk<'ctx>>>,
    pub result: Option<&'ctx Name>,
    pub blocks: HashMap<&'ctx Name, &'ctx BasicBlock>,
    pub ectx: EvalCtx,
}

enum DecodeResult<'ctx> {
    Jump(&'ctx Name),
    Return(Rc<Thunk<'ctx>>),
}

impl<'ctx> Func<'ctx> for InterpFunc<'ctx> {
    fn name(&self) -> &'ctx str {
        self.src.name.as_str()
    }

    fn call_imp<'a>(&'a self, args: ExecArgs<'ctx, 'a>) -> LocalBoxFuture<'a, Rc<Thunk<'ctx>>> {
        Box::pin(InterpFrame::call(args.ctx.clone(), self.ectx, args.data, args.backtrace, self.src, args.args))
    }
}

impl<'ctx> InterpFrame<'ctx> {
    pub async fn call<'a>(ctx: Rc<Ctx<'ctx>>,
                          ectx: EvalCtx,
                          data: &'a DataFlow<'ctx>,
                          backtrace: &'a Backtrace<'ctx>,
                          fun: &'ctx Function,
                          params: Vec<Rc<Thunk<'ctx>>>) -> Rc<Thunk<'ctx>> {
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
            ctx,
            fun,
            origin: None,
            temps,
            allocs: vec![],
            result: None,
            blocks,
            ectx,
        };
        frame.decode(data, backtrace).await
    }
    pub fn decode<'a>(&'a mut self, data: &'a DataFlow<'ctx>, backtrace: &'a Backtrace<'ctx>) -> impl Future<Output=Rc<Thunk<'ctx>>> + 'a {
        Box::pin(async move { self.decode_impl(data, backtrace).await })
    }
    pub async fn decode_impl(&mut self, data: &DataFlow<'ctx>, backtrace: &Backtrace<'ctx>) -> Rc<Thunk<'ctx>> {
        let mut block = self.fun.basic_blocks.get(0).unwrap();
        loop {
            for instr in block.instrs.iter() {
                self.decode_instr(data, &backtrace.prepend(BacktraceFrame { name: &*self.fun.name }), instr).await;
            }
            block = match self.decode_term(data, &backtrace.prepend(BacktraceFrame { name: &*self.fun.name }), &block.term).await {
                DecodeResult::Jump(name) => *self.blocks.get(name).unwrap(),
                DecodeResult::Return(result) => return result,
            };
        }
    }
    async fn decode_instr<'a>(&'a mut self, data: &'a DataFlow<'ctx>, backtrace: &'a Backtrace<'ctx>, instr: &'ctx Instruction) {
        match instr {
            Instruction::Phi(phi) => {
                let (oper, _) =
                    phi.incoming_values.iter()
                        .find(|(_, name)| Some(name) == self.origin).unwrap();
                let deps = vec![self.get_temp(data, oper).await];
                self.add_thunk(data, Some(&phi.dest), deps, |args| args.args[0].clone()).await;
            }
            Instruction::Call(call) => {
                self.decode_call(data, backtrace, &call.function, &call.arguments, call.dest.as_ref()).await;
            }
            Instruction::Alloca(alloca) => {
                let num = self.get_temp(data, &alloca.num_elements).await;
                let layout = self.ctx.layout(&alloca.allocated_type);
                let thunk = self.add_thunk(
                    data,
                    Some(&alloca.dest),
                    vec![num],
                    move |args| {
                        let layout = layout.repeat(args.args[0].as_u64());
                        args.process.alloc(args.tctx, layout)
                    },
                ).await;
                self.allocs.push(thunk);
            }
            Instruction::Store(store) => {
                let deps = vec![
                    self.get_temp(data, &store.address).await,
                    self.get_temp(data, &store.value).await];

                self.add_thunk(data, None, deps, move |args| {
                    args.process.store(args.tctx, args.args[0], args.args[1], store.atomicity.as_ref());
                    Value::from(())
                }).await;
            }
            Instruction::Load(load) => {
                let deps = vec![self.get_temp(data, &load.address).await];
                let layout = self.ctx.layout(&self.ctx.target_of(&self.ctx.type_of(self.ectx, &load.address)));
                self.add_thunk(data, Some(&load.dest), deps, move |args| {
                    args.process.load(args.tctx, args.args[0], layout, load.atomicity.as_ref())
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
                let deps = vec![self.get_temp(data, operand0).await, self.get_temp(data, operand1).await];
                let ctx = self.ctx.clone();
                self.add_thunk(data, Some(dest), deps, move |args|
                    Self::binary(&*ctx, instr, args.args[0], args.args[1])).await;
            }
            Instruction::SExt(SExt { dest, operand, .. }) |
            Instruction::ZExt(ZExt { dest, operand, .. }) |
            Instruction::Trunc(Trunc { dest, operand, .. }) |
            Instruction::PtrToInt(PtrToInt { dest, operand, .. }) |
            Instruction::IntToPtr(IntToPtr { dest, operand, .. }) |
            Instruction::BitCast(BitCast { dest, operand, .. }) => {
                let deps = vec![self.get_temp(data, operand).await];
                let ctx = self.ctx.clone();
                self.add_thunk(data, Some(dest), deps,
                               move |args| Self::unary(&*ctx, instr, args.args[0])).await;
            }
            Instruction::GetElementPtr(gep) => {
                let mut deps = vec![self.get_temp(data, &gep.address).await];
                for ind in gep.indices.iter() {
                    deps.push(self.get_temp(data, ind).await);
                }
                let ectx = self.ectx;
                let ty = self.ctx.type_of(self.ectx, &gep.address);
                let ctx = self.ctx.clone();
                self.add_thunk(data, Some(&gep.dest), deps, move |args| {
                    let (_, offset) = ctx.offset_of(&ty,
                                                    args.args[1..].iter().map(|v| v.as_i64()));
                    ctx.value_from_address(add_u64_i64(args.args[0].as_u64(), offset))
                }).await;
            }
            Instruction::InsertValue(InsertValue { aggregate, element, dest, indices, .. }) => {
                let deps = vec![self.get_temp(data, aggregate).await, self.get_temp(data, element).await];
                let ty = self.ctx.type_of(self.ectx, aggregate);
                let (_, offset) = self.ctx.offset_of(&ty, indices.iter().map(|i| *i as i64));
                let offset = offset as usize;
                self.add_thunk(data, Some(dest), deps, move |args| {
                    let mut aggregate = args.args[0].clone();
                    let element = args.args[1];
                    let element = element.bytes();
                    aggregate.bytes_mut()[offset..(offset + element.len())].copy_from_slice(element);
                    aggregate
                }).await;
            }
            Instruction::AtomicRMW(AtomicRMW { address, value, dest, operation, atomicity, .. }) => {
                let deps = vec![self.get_temp(data, address).await, self.get_temp(data, value).await];
                let ty = self.ctx.target_of(&self.ctx.type_of(self.ectx, address));
                let layout = self.ctx.layout(&ty);
                self.add_thunk(data, Some(dest), deps, move |args| {
                    let current = args.process.load(args.tctx, args.args[0], layout, Some(atomicity)).clone();
                    let new = match operation {
                        RMWBinOp::Add => &current + args.args[1],
                        RMWBinOp::Sub => &current - args.args[1],
                        RMWBinOp::Xchg => args.args[1].clone(),
                        RMWBinOp::And => &current & args.args[1],
                        RMWBinOp::Or => &current | args.args[1],
                        RMWBinOp::Xor => &current ^ args.args[1],
                        _ => todo!("{:?}", operation),
                    };
                    args.process.store(args.tctx, args.args[0], &new, Some(atomicity));
                    current
                }).await;
            }
            Instruction::Select(Select { condition, true_value, false_value, dest, .. }) => {
                let deps =
                    vec![self.get_temp(data, condition).await,
                         self.get_temp(data, true_value).await,
                         self.get_temp(data, false_value).await];
                self.add_thunk(data, Some(dest), deps, |args|
                    if args.args[0].unwrap_bool() {
                        args.args[1].clone()
                    } else {
                        args.args[2].clone()
                    },
                ).await;
            }
            Instruction::ExtractValue(ExtractValue { aggregate, dest, indices, .. }) => {
                let deps = vec![self.get_temp(data, aggregate).await];
                let ty = self.ctx.type_of(self.ectx, aggregate);
                let (ty2, offset) = self.ctx.offset_of(&ty, indices.iter().map(|i| *i as i64));
                let layout = self.ctx.layout(&ty2);
                let offset = offset as usize;
                self.add_thunk(data, Some(dest), deps, move |args| {
                    let bytes = &args.args[0].bytes()[offset..offset + layout.bytes() as usize];
                    Value::from_bytes(bytes, layout)
                }).await;
            }
            Instruction::CmpXchg(CmpXchg { address, expected, replacement, dest, atomicity, .. }) => {
                let deps =
                    vec![self.get_temp(data, address).await,
                         self.get_temp(data, expected).await,
                         self.get_temp(data, replacement).await];
                let ty = self.ctx.target_of(&self.ctx.type_of(self.ectx, address));
                let layout = self.ctx.layout(&ty);
                self.add_thunk(data, Some(dest), deps, move |args| {
                    let (address, expected, replacement) = (args.args[0], args.args[1], args.args[2]);
                    let old = args.process.load(args.tctx, address, layout, Some(atomicity));
                    let success = &old == expected;
                    if success {
                        args.process.store(args.tctx, address, replacement, Some(atomicity));
                    }
                    Value::aggregate(vec![old.clone(), Value::from(success)].into_iter(), false)
                }).await;
            }
            Instruction::Fence(Fence { atomicity, .. }) => {
                //TODO
            }
            _ => todo!("{:?}", instr),
        }
    }
    fn unary(ctx: &Ctx<'ctx>, instr: &Instruction, value: &Value) -> Value {
        match instr {
            Instruction::SExt(SExt { to_type, .. }) =>
                value.sext(ctx.layout(to_type).bits()),
            Instruction::ZExt(ZExt { to_type, .. }) =>
                value.zext(ctx.layout(to_type).bits()),
            Instruction::Trunc(Trunc { to_type, .. }) |
            Instruction::PtrToInt(PtrToInt { to_type, .. }) |
            Instruction::IntToPtr(IntToPtr { to_type, .. }) |
            Instruction::BitCast(BitCast { to_type, .. }) => {
                let to_bits = ctx.layout(to_type).bits();
                if to_bits < value.bits() {
                    value.truncate(to_bits)
                } else {
                    value.zext(to_bits)
                }
            }
            _ => unreachable!()
        }
    }
    fn binary(ctx: &Ctx<'ctx>, instr: &Instruction, x: &Value, y: &Value) -> Value {
        match instr {
            Instruction::Add(Add { .. }) => x + y,
            Instruction::Sub(Sub { .. }) => x - y,
            Instruction::Mul(Mul { .. }) => x * y,
            Instruction::UDiv(UDiv { .. }) => x / y,
            Instruction::SDiv(SDiv { .. }) => x.sdiv(y),
            Instruction::URem(URem { .. }) => x % y,
            Instruction::SRem(SRem { .. }) => x.srem(y),
            Instruction::Xor(Xor { .. }) => x ^ y,
            Instruction::And(And { .. }) => x & y,
            Instruction::Or(Or { .. }) => x | y,
            Instruction::Shl(Shl { .. }) => x << y,
            Instruction::LShr(LShr { .. }) => x >> y,
            Instruction::AShr(AShr { .. }) => x.sshr(y),
            Instruction::ICmp(ICmp { predicate, .. }) => Value::from(Self::icmp(predicate, x, y)),
            _ => todo!(),
        }
    }
    fn icmp(predicate: &IntPredicate, x: &Value, y: &Value) -> bool {
        match predicate {
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
        }
    }
    async fn decode_term<'a>(&'a mut self, data: &'a DataFlow<'ctx>, backtrace: &'a Backtrace<'ctx>, term: &'ctx Terminator) -> DecodeResult<'ctx> {
        match term {
            Terminator::Br(br) => {
                DecodeResult::Jump(&br.dest)
            }
            Terminator::Ret(ret) => {
                for x in self.allocs.iter() {
                    data.add_thunk(vec![x.clone()], |args| {
                        args.process.free(args.tctx, &args.args[0]);
                        Value::from(())
                    }).await;
                }
                let result = if let Some(oper) = ret.return_operand.as_ref() {
                    self.get_temp(data, oper).await
                } else {
                    self.add_thunk(data, None, vec![], |args| Value::from(())).await
                };
                DecodeResult::Return(result)
            }
            Terminator::CondBr(condbr) => {
                if self.get_temp(data, &condbr.condition).await.get().await.unwrap_bool() {
                    DecodeResult::Jump(&condbr.true_dest)
                } else {
                    DecodeResult::Jump(&condbr.false_dest)
                }
            }
            Terminator::Unreachable(_) => {
                panic!("Unreachable");
            }
            Terminator::Invoke(invoke) => {
                self.decode_call(data, backtrace, &invoke.function, &invoke.arguments, Some(&invoke.result)).await;
                DecodeResult::Jump(&invoke.return_label)
            }
            Terminator::Switch(switch) => {
                let cond = self.get_temp(data, &switch.operand).await;
                let cond = cond.get().await;
                let mut target = None;
                for (pat, dest) in switch.dests.iter() {
                    let (_, pat) = self.ctx.get_constant(self.ectx, &pat);
                    if &pat == cond {
                        target = Some(dest);
                    }
                }
                DecodeResult::Jump(target.unwrap_or(&switch.default_dest))
            }
            term => todo!("{:?}", term)
        }
    }
    async fn get_temp<'a>(&'a mut self, data: &'a DataFlow<'ctx>, oper: &'ctx Operand) -> Rc<Thunk<'ctx>> {
        match oper {
            Operand::LocalOperand { name, .. } => {
                self.temps.get(name).unwrap_or_else(|| panic!("No variable named {:?} in \n{:#?}", name, self)).clone()
            }
            Operand::ConstantOperand(constant) => {
                let value = self.ctx.get_constant(self.ectx, constant).1;
                self.add_thunk(data, None, vec![], move |_| value).await
            }
            Operand::MetadataOperand => {
                self.add_thunk(data, None, vec![], |_| Value::from(())).await
            }
        }
    }
    async fn decode_call<'a>(&'a mut self,
                             data: &'a DataFlow<'ctx>,
                             backtrace: &'a Backtrace<'ctx>,
                             function: &'ctx Either<InlineAssembly, Operand>,
                             arguments: &'ctx Vec<(Operand, Vec<ParameterAttribute>)>,
                             dest: Option<&'ctx Name>,
    ) {
        let fun = match function {
            Either::Left(assembly) => todo!("{:?}", assembly),
            Either::Right(fun) => fun,
        };
        let fun = self.get_temp(data, fun).await;
        let fun = fun.get().await;
        let fun = self.ctx.reverse_lookup(fun);
        let fun = self.ctx.functions.get(&fun).cloned().unwrap_or_else(|| panic!("Unknown function {:?}", fun));
        let mut deps = vec![];
        for (oper, _) in arguments.iter() {
            deps.push(self.get_temp(data, oper).await)
        }
        let result = fun.call_imp(ExecArgs { ctx: &self.ctx, data, backtrace, args: deps }).await;
        if let Some(dest) = dest {
            self.temps.insert(dest, result);
        }
        // self.add_thunk(data, dest, vec![result], |args| {
        //     args.args.get(0).cloned().cloned().unwrap_or(Value::from(()))
        // }).await;
    }
    async fn add_thunk<'a>(&'a mut self,
                           data: &'a DataFlow<'ctx>,
                           dest: Option<&'ctx Name>,
                           deps: Vec<Rc<Thunk<'ctx>>>,
                           compute: impl 'ctx + FnOnce(ComputeArgs<'ctx, '_>) -> Value) -> Rc<Thunk<'ctx>> {
        let thunk = data.add_thunk(deps, compute).await;
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

