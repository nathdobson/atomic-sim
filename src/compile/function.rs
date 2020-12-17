use llvm_ir::{Function, Module, Name, Instruction, Terminator, Operand, BasicBlock, ConstantRef, Constant, IntPredicate, DebugLoc, HasDebugLoc};
use std::rc::Rc;
use llvm_ir::instruction::*;
use crate::symbols::{Symbol, SymbolTable, SymbolDef};
use crate::value::{Value, add_u64_i64};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use crate::compile::class::{TypeMap, Class, ClassKind, VectorClass};
use crate::layout::{Layout, Packing};
use crate::memory::Memory;
use crate::process::Process;
use crate::thread::ThreadId;
use llvm_ir::constant;
use llvm_ir::terminator::*;
use either::Either;
use itertools::Itertools;
use crate::compile::operation::{COperationName, COperation, CFPPredicate, OperCompiler};
use crate::compile::operation::CIntPredicate;
use llvm_ir::constant::Float;
use llvm_ir::types::FPType;
use crate::function::Func;
use crate::flow::FlowCtx;
use crate::data::{Thunk, DataFlow, ThunkInner, ThunkState};
use crate::backtrace::BacktraceFrame;
use std::cell::RefCell;
use crate::util::lazy::Lazy;
use llvm_ir::module::ThreadLocalMode;
use crate::compile::expr::CExpr;
use crate::compile::module::{ModuleId, ModuleCompiler};
use crate::compile::expr::ExprCompiler;

#[derive(Debug)]
pub struct CFunc {
    pub fp: u64,
    pub src: Function,
    pub args: Vec<CLocal>,
    pub blocks: Vec<CBlock>,
    pub locals: Vec<Name>,
}

#[derive(Debug)]
pub struct CBlockInstr {
    pub instr: CInstr,
    pub frame: BacktraceFrame,
}

#[derive(Debug)]
pub struct CBlockTerm {
    pub term: CTerm,
    pub frame: BacktraceFrame,
}

#[derive(Debug)]
pub struct CBlock {
    pub id: CBlockId,
    pub phis: Vec<CPhi>,
    pub instrs: Vec<CBlockInstr>,
    pub term: CBlockTerm,
}

#[derive(Debug)]
pub struct CLoad {
    pub dest: CLocal,
    pub address: COperand,
    pub atomicity: Option<Atomicity>,
}

#[derive(Debug)]
pub struct CStore {
    pub address: COperand,
    pub value: COperand,
    pub atomicity: Option<Atomicity>,
}


#[derive(Debug)]
pub struct CCompute {
    pub dest: CLocal,
    pub operands: Vec<COperand>,
    pub operation: COperation,
}

#[derive(Debug)]
pub struct CAlloca {
    pub dest: CLocal,
    pub count: COperand,
    pub layout: Layout,
}

#[derive(Debug)]
pub struct CCall {
    pub dest: Option<CLocal>,
    pub func: COperand,
    pub arguments: Vec<COperand>,
}

#[derive(Debug)]
pub struct CAtomicRMW {
    pub dest: CLocal,
    pub address: COperand,
    pub value: COperand,
    pub operation: COperation,
    pub atomicity: Atomicity,
}

#[derive(Debug)]
pub struct CCmpXchg {
    pub dest: CLocal,
    pub address: COperand,
    pub expected: COperand,
    pub replacement: COperand,
    pub success: MemoryOrdering,
    pub failure: MemoryOrdering,
    pub weak: bool,
}


#[derive(Debug)]
pub enum CInstr {
    Compute(Rc<CCompute>),

    Call(Rc<CCall>),
    Alloca(Rc<CAlloca>),

    Load(Rc<CLoad>),
    Store(Rc<CStore>),

    CmpXchg(Rc<CCmpXchg>),
    AtomicRMW(Rc<CAtomicRMW>),
    Freeze,
    Fence,

    Unknown(Instruction),
}

#[derive(Debug)]
pub struct CRet {
    pub return_operand: COperand,
}

#[derive(Debug)]
pub struct CBr {
    pub target: CBlockId,
}

#[derive(Debug)]
pub struct CCondBr {
    pub condition: COperand,
    pub true_target: CBlockId,
    pub false_target: CBlockId,
}

#[derive(Debug)]
pub struct CSwitch {
    pub condition: COperand,
    pub targets: HashMap<Value, CBlockId>,
    pub default_target: CBlockId,
}

#[derive(Debug)]
pub struct CInvoke {
    pub function: COperand,
    pub arguments: Vec<COperand>,
    pub dest: CLocal,
    pub target: CBlockId,
}

#[derive(Debug)]
pub struct CUnreachable {}

#[derive(Debug)]
pub enum CTerm {
    Ret(CRet),
    Br(CBr),
    CondBr(CCondBr),
    Switch(CSwitch),
    Invoke(CInvoke),
    Unreachable(CUnreachable),
    Resume,
}


#[derive(Debug)]
pub enum COperand {
    Constant(Class, Thunk),
    Expr(CExpr),
    Local(Class, CLocal),
    Inline,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CBlockId(pub usize);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CLocal(pub usize);

#[derive(Debug)]
pub struct CPhi {
    pub dest: CLocal,
    pub mapping: HashMap<CBlockId, COperand>,
}

pub struct FuncCompiler {
    process: Process,
    expr_compiler: Rc<ExprCompiler>,
    oper_compiler: Rc<OperCompiler>,
    locals: HashMap<Name, CLocal>,
    local_names: Vec<Name>,
    block_ids: HashMap<Name, CBlockId>,
    type_map: TypeMap,
}


impl FuncCompiler {
    pub fn new(process: Process,
               expr_compiler: Rc<ExprCompiler>,
               oper_compiler: Rc<OperCompiler>) -> Self {
        FuncCompiler {
            process: process.clone(),
            expr_compiler,
            oper_compiler,
            locals: HashMap::new(),
            local_names: vec![],
            block_ids: HashMap::new(),
            type_map: process.types(),
        }
    }
    fn compile_local(&mut self, name: &Name) -> CLocal {
        let local_names = &mut self.local_names;
        *self.locals.entry(name.clone()).or_insert_with(|| {
            let local = CLocal(local_names.len());
            local_names.push(name.clone());
            local
        })
    }

    fn compile_const(&mut self, constant: &ConstantRef) -> CExpr {
        self.expr_compiler.compile_const(constant)
    }
    fn compile_oper(&mut self, oper: COperationName, input: &[&Class]) -> COperation {
        self.oper_compiler.compile_operation(oper, input)
    }

    fn constant_thunk(&mut self, value: Value) -> Thunk {
        Thunk::constant(self.process.clone(), value)
    }

    fn compile_operand(&mut self, operand: &Operand) -> COperand {
        match operand {
            Operand::LocalOperand { name, ty } =>
                COperand::Local(self.type_map.get(ty), self.compile_local(name)),
            Operand::ConstantOperand(expr) => {
                match self.compile_const(expr) {
                    CExpr::Const { class, value } =>
                        COperand::Constant(class, self.constant_thunk(value)),
                    cexpr =>
                        COperand::Expr(cexpr)
                }
            }
            Operand::MetadataOperand =>
                COperand::Constant(self.type_map.void(), self.constant_thunk(Value::from(()))),
        }
    }

    fn compile_global_name(&mut self, name: &Name) -> Symbol {
        todo!()
    }
    fn compile_phi(&mut self, phi: &Phi) -> CPhi {
        CPhi {
            dest: self.compile_local(&phi.dest),
            mapping: phi.incoming_values.iter().map(|(oper, origin)| {
                (*self.block_ids.get(&origin).unwrap(), self.compile_operand(oper))
            }).collect(),
        }
    }
    fn compile_binop(&mut self, instr: &Instruction) -> COperationName {
        use COperationName::*;
        match instr {
            Instruction::Add(_) => Add,
            Instruction::Sub(_) => Sub,
            Instruction::Mul(_) => Mul,
            Instruction::UDiv(_) => UDiv,
            Instruction::SDiv(_) => SDiv,
            Instruction::URem(_) => URem,
            Instruction::SRem(_) => SRem,
            Instruction::And(_) => And,
            Instruction::Or(_) => Or,
            Instruction::Xor(_) => Xor,
            Instruction::Shl(_) => Shl,
            Instruction::LShr(_) => LShr,
            Instruction::AShr(_) => AShr,
            Instruction::FAdd(_) => FAdd,
            Instruction::FSub(_) => FSub,
            Instruction::FMul(_) => FMul,
            Instruction::FDiv(_) => FDiv,
            Instruction::FRem(_) => FRem,
            _ => unreachable!(),
        }
    }
    fn compile_cast(&mut self, instr: &Instruction, target: Class) -> COperationName {
        use COperationName::*;
        match instr {
            Instruction::Trunc(_) => Trunc(target),
            Instruction::ZExt(_) => ZExt(target),
            Instruction::SExt(_) => SExt(target),
            Instruction::FPTrunc(_) => FPTrunc(target),
            Instruction::FPExt(_) => FPExt(target),
            Instruction::FPToUI(_) => FPToUI(target),
            Instruction::FPToSI(_) => FPToSI(target),
            Instruction::UIToFP(_) => UIToFP(target),
            Instruction::SIToFP(_) => SIToFP(target),
            Instruction::PtrToInt(_) => ZExt(target),
            Instruction::IntToPtr(_) => ZExt(target),
            Instruction::AddrSpaceCast(_) => ZExt(target),
            Instruction::BitCast(_) => BitCast(target),
            _ => unreachable!()
        }
    }
    fn compile_compute(&mut self, dest: &Name, operands: &[&Operand], name: COperationName) -> CInstr {
        let operands =
            operands.iter()
                .map(|oper| self.compile_operand(oper))
                .collect::<Vec<_>>();
        let operand_classes =
            operands.iter()
                .map(|oper| oper.class())
                .collect::<Vec<_>>();
        let operation = self.oper_compiler.compile_operation(name, &operand_classes);
        CInstr::Compute(Rc::new(CCompute {
            dest: self.compile_local(dest),
            operands,
            operation,
        }))
    }
    fn compile_instr(&mut self, instr: &Instruction) -> CInstr {
        match instr {
            Instruction::Add(Add { operand0, operand1, dest, debugloc })
            | Instruction::Sub(Sub { operand0, operand1, dest, debugloc })
            | Instruction::Mul(Mul { operand0, operand1, dest, debugloc })
            | Instruction::UDiv(UDiv { operand0, operand1, dest, debugloc })
            | Instruction::SDiv(SDiv { operand0, operand1, dest, debugloc })
            | Instruction::URem(URem { operand0, operand1, dest, debugloc })
            | Instruction::SRem(SRem { operand0, operand1, dest, debugloc })
            | Instruction::And(And { operand0, operand1, dest, debugloc })
            | Instruction::Or(Or { operand0, operand1, dest, debugloc })
            | Instruction::Xor(Xor { operand0, operand1, dest, debugloc })
            | Instruction::Shl(Shl { operand0, operand1, dest, debugloc })
            | Instruction::LShr(LShr { operand0, operand1, dest, debugloc })
            | Instruction::AShr(AShr { operand0, operand1, dest, debugloc })
            | Instruction::FAdd(FAdd { operand0, operand1, dest, debugloc })
            | Instruction::FSub(FSub { operand0, operand1, dest, debugloc })
            | Instruction::FMul(FMul { operand0, operand1, dest, debugloc })
            | Instruction::FDiv(FDiv { operand0, operand1, dest, debugloc })
            | Instruction::FRem(FRem { operand0, operand1, dest, debugloc })
            => {
                let operand0 = self.compile_operand(operand0);
                let operand1 = self.compile_operand(operand1);
                let class = operand0.class().clone();
                assert_eq!(&class, operand1.class(), "{}", instr);
                CInstr::Compute(Rc::new(CCompute {
                    dest: self.compile_local(dest),
                    operands: vec![operand0, operand1],
                    operation: COperation {
                        input: vec![class.clone(), class.clone()],
                        output: class,
                        name: self.compile_binop(instr),
                    },
                }))
            }
            Instruction::FNeg(FNeg { operand, dest, debugloc })
            => {
                self.compile_compute(dest, &[operand], COperationName::FNeg)
            }
            Instruction::ZExt(ZExt { operand, to_type, dest, debugloc })
            | Instruction::SExt(SExt { operand, to_type, dest, debugloc })
            | Instruction::Trunc(Trunc { operand, to_type, dest, debugloc })
            | Instruction::PtrToInt(PtrToInt { operand, to_type, dest, debugloc })
            | Instruction::IntToPtr(IntToPtr { operand, to_type, dest, debugloc })
            | Instruction::AddrSpaceCast(AddrSpaceCast { operand, to_type, dest, debugloc })
            | Instruction::BitCast(BitCast { operand, to_type, dest, debugloc })
            | Instruction::FPTrunc(FPTrunc { operand, to_type, dest, debugloc })
            | Instruction::FPExt(FPExt { operand, to_type, dest, debugloc })
            | Instruction::FPToUI(FPToUI { operand, to_type, dest, debugloc })
            | Instruction::FPToSI(FPToSI { operand, to_type, dest, debugloc })
            | Instruction::UIToFP(UIToFP { operand, to_type, dest, debugloc })
            | Instruction::SIToFP(SIToFP { operand, to_type, dest, debugloc })
            => {
                let target = self.type_map.get(to_type);
                let name = self.compile_cast(instr, target);
                self.compile_compute(dest, &[operand], name)
            }
            Instruction::ICmp(ICmp { predicate, operand0, operand1, dest, debugloc })
            => {
                self.compile_compute(dest,
                                     &[operand0, operand1],
                                     COperationName::ICmp(CIntPredicate::from(*predicate)))
            }
            Instruction::FCmp(FCmp { predicate, operand0, operand1, dest, debugloc })
            => {
                self.compile_compute(dest,
                                     &[operand0, operand1],
                                     COperationName::FCmp(CFPPredicate::from(*predicate)))
            }
            Instruction::GetElementPtr(GetElementPtr { address, indices, dest, in_bounds, debugloc }) => {
                let operand = self.compile_operand(address);
                let mut output = operand.class().clone();
                let mut operands = vec![operand];
                let mut input = vec![output.clone()];
                for index in indices {
                    let operand = self.compile_operand(index);
                    match &operand {
                        COperand::Constant(_, c) =>
                            output = output.element(c.try_get().unwrap().as_i64()).class,
                        COperand::Local(_, _) =>
                            output = output.element(-1).class,
                        _ => todo!(),
                    }
                    input.push(operand.class().clone());
                    operands.push(operand);
                }
                CInstr::Compute(Rc::new(CCompute {
                    dest: self.compile_local(dest),
                    operands,
                    operation: COperation {
                        input,
                        output: self.type_map.ptr(output),
                        name: COperationName::GetElementPtr,
                    },
                }))
            }
            Instruction::ExtractElement(ExtractElement { vector, index, dest, debugloc }) => {
                self.compile_compute(dest, &[vector, index], COperationName::ExtractElement)
            }
            Instruction::InsertElement(InsertElement { vector, element, index, dest, debugloc }) => {
                self.compile_compute(dest, &[vector, element, index], COperationName::InsertElement)
            }
            Instruction::ShuffleVector(ShuffleVector { operand0, operand1, dest, mask, debugloc }) => {
                let mask = self.compile_const(mask);
                let (mask_class, mask) = mask.as_const().unwrap();
                let mask_vector = mask_class.as_vector().unwrap();
                let indices =
                    (0..mask_vector.len as i64)
                        .map(|i| mask.extract(&mask_class, i).as_u64() as i64)
                        .collect::<Vec<_>>();
                self.compile_compute(dest, &[operand0, operand1], COperationName::Shuffle(indices))
            }
            Instruction::ExtractValue(ExtractValue { aggregate, indices, dest, debugloc }) => {
                self.compile_compute(dest, &[aggregate],
                                     COperationName::ExtractValue(indices.iter().map(|x| *x as i64).collect()))
            }
            Instruction::InsertValue(InsertValue { aggregate, element, indices, dest, debugloc }) => {
                self.compile_compute(dest,
                                     &[aggregate, element],
                                     COperationName::InsertValue(
                                         indices.iter().map(|x| *x as i64).collect()))
            }
            Instruction::Select(Select { condition, true_value, false_value, dest, debugloc }) => {
                self.compile_compute(dest, &[condition, true_value, false_value], COperationName::Select)
            }
            Instruction::Phi(Phi { incoming_values, dest, to_type, debugloc }) => {
                unreachable!()
            }
            Instruction::Alloca(Alloca { allocated_type, num_elements, dest, alignment, debugloc }) => {
                CInstr::Alloca(Rc::new(CAlloca {
                    dest: self.compile_local(dest),
                    count: self.compile_operand(num_elements),
                    layout: self.type_map.get(allocated_type).layout().align_to_bits((8 * *alignment) as u64),
                }))
            }
            Instruction::Load(Load { address, dest, volatile, atomicity, alignment, debugloc }) => {
                CInstr::Load(Rc::new(CLoad {
                    dest: self.compile_local(dest),
                    address: self.compile_operand(address),
                    atomicity: atomicity.clone(),
                }))
            }
            Instruction::Store(Store { address, value, volatile, atomicity, alignment, debugloc }) => {
                CInstr::Store(Rc::new(CStore {
                    address: self.compile_operand(address),
                    value: self.compile_operand(value),
                    atomicity: atomicity.clone(),
                }))
            }
            Instruction::CmpXchg(CmpXchg {
                                     address,
                                     expected,
                                     replacement,
                                     dest,
                                     volatile,
                                     atomicity,
                                     failure_memory_ordering,
                                     debugloc,
                                     weak
                                 }) => {
                let address = self.compile_operand(address);
                let expected = self.compile_operand(expected);
                let replacement = self.compile_operand(replacement);
                CInstr::CmpXchg(Rc::new(CCmpXchg {
                    dest: self.compile_local(dest),
                    address,
                    expected,
                    replacement,
                    success: atomicity.mem_ordering,
                    failure: *failure_memory_ordering,
                    weak: *weak,
                }))
            }
            Instruction::AtomicRMW(AtomicRMW {
                                       address,
                                       value,
                                       dest,
                                       operation,
                                       volatile,
                                       atomicity,
                                       debugloc
                                   }) => {
                let address = self.compile_operand(address);
                let value = self.compile_operand(value);
                let class = value.class().clone();
                let operation = match operation {
                    RMWBinOp::Xchg => COperationName::Xchg,
                    RMWBinOp::Add => COperationName::Add,
                    RMWBinOp::Sub => COperationName::Sub,
                    RMWBinOp::And => COperationName::And,
                    RMWBinOp::Or => COperationName::Or,
                    RMWBinOp::Xor => COperationName::Xor,
                    RMWBinOp::FAdd => COperationName::FAdd,
                    RMWBinOp::FSub => COperationName::FSub,
                    _ => todo!("{:?}", operation),
                };
                let operation = self.compile_oper(operation, &[&class, &class]);
                CInstr::AtomicRMW(Rc::new(CAtomicRMW {
                    dest: self.compile_local(dest),
                    address,
                    value,
                    operation,
                    atomicity: atomicity.clone(),
                }))
            }
            Instruction::Freeze(Freeze { operand, dest, debugloc }) => {
                CInstr::Freeze
            }
            Instruction::Call(Call {
                                  function,
                                  arguments,
                                  return_attributes,
                                  dest,
                                  function_attributes,
                                  is_tail_call,
                                  calling_convention,
                                  debugloc
                              }) => {
                let dest = dest.as_ref().map(|dest| self.compile_local(dest));
                let func = self.compile_func_operand(function);
                let arguments =
                    arguments.iter()
                        .map(|(arg, _)| self.compile_operand(arg))
                        .collect::<Vec<_>>();
                CInstr::Call(Rc::new(CCall {
                    dest,
                    func,
                    arguments,
                }))
            }
            Instruction::Fence(Fence { atomicity, debugloc }) => {
                CInstr::Fence
            }
            instr => CInstr::Unknown(instr.clone())
        }
    }
    fn compile_func_operand(&mut self, oper: &Either<InlineAssembly, Operand>) -> COperand {
        match oper {
            Either::Left(_) => COperand::Inline,
            Either::Right(oper) => self.compile_operand(oper),
        }
    }
    fn compile_target(&mut self, name: &Name) -> CBlockId {
        *self.block_ids.get(name).unwrap()
    }
    fn compile_term(&mut self, term: &Terminator) -> CTerm {
        match term {
            Terminator::Ret(Ret { return_operand, debugloc }) => {
                let return_operand = if let Some(return_operand) = return_operand {
                    self.compile_operand(return_operand)
                } else {
                    COperand::Constant(self.type_map.void(), self.constant_thunk(Value::from(())))
                };
                CTerm::Ret(CRet { return_operand })
            }
            Terminator::Br(Br { dest, debugloc }) => {
                CTerm::Br(CBr {
                    target: self.compile_target(dest),
                })
            }
            Terminator::CondBr(CondBr {
                                   condition,
                                   true_dest,
                                   false_dest,
                                   debugloc
                               }) => {
                CTerm::CondBr(CCondBr {
                    condition: self.compile_operand(condition),
                    true_target: self.compile_target(true_dest),
                    false_target: self.compile_target(false_dest),
                })
            }
            Terminator::Switch(Switch {
                                   operand,
                                   dests,
                                   default_dest,
                                   debugloc
                               }) => {
                CTerm::Switch(CSwitch {
                    condition: self.compile_operand(operand),
                    targets: dests.iter().map(|(pat, target)| {
                        (self.compile_const(pat).as_const().unwrap().1.clone(), self.compile_target(target))
                    }).collect(),
                    default_target: self.compile_target(default_dest),
                })
            }
            Terminator::IndirectBr(IndirectBr {
                                       operand,
                                       possible_dests,
                                       debugloc
                                   }) => {
                todo!()
            }
            Terminator::Invoke(Invoke {
                                   function,
                                   arguments,
                                   return_attributes,
                                   result,
                                   return_label,
                                   exception_label,
                                   function_attributes,
                                   calling_convention,
                                   debugloc
                               }) => {
                CTerm::Invoke(CInvoke {
                    function: self.compile_operand(
                        function.as_ref().expect_right("Inline assembly!")),
                    arguments: arguments.iter().map(|(arg, _)|
                        self.compile_operand(arg)
                    ).collect(),
                    dest: self.compile_local(result),
                    target: self.compile_target(return_label),
                })
            }
            Terminator::Resume(Resume { operand, debugloc }) => {
                CTerm::Resume
            }
            Terminator::Unreachable(Unreachable { debugloc }) => {
                CTerm::Unreachable(CUnreachable {})
            }
            Terminator::CleanupRet(CleanupRet {
                                       cleanup_pad,
                                       unwind_dest,
                                       debugloc
                                   }) => {
                todo!()
            }
            Terminator::CatchRet(CatchRet {
                                     catch_pad,
                                     successor,
                                     debugloc
                                 }) => {
                todo!()
            }
            Terminator::CatchSwitch(CatchSwitch {
                                        parent_pad,
                                        catch_handlers,
                                        default_unwind_dest,
                                        result,
                                        debugloc
                                    }) => {
                todo!()
            }
            Terminator::CallBr(CallBr {
                                   function,
                                   arguments,
                                   return_attributes,
                                   result, return_label,
                                   other_labels,
                                   function_attributes,
                                   calling_convention,
                                   debugloc
                               }) => {
                todo!()
            }
        }
    }
    fn compile_block(&mut self, loc: &Value, id: CBlockId, block: &BasicBlock) -> CBlock {
        let mut phis = vec![];
        let mut instrs = vec![];
        for instr in block.instrs.iter() {
            match instr {
                Instruction::Phi(phi) => phis.push(self.compile_phi(&phi)),
                instr => instrs.push(CBlockInstr {
                    instr: self.compile_instr(&instr),
                    frame: BacktraceFrame::new(loc.as_u64(), format!("{} {:?}", instr, instr.get_debug_loc())),
                }),
            }
        }
        let term = CBlockTerm {
            term: self.compile_term(&block.term),
            frame: BacktraceFrame::new(loc.as_u64(), format!("{} {:?}", block.term, block.term.get_debug_loc())),
        };
        CBlock { id, phis, instrs, term }
    }
    pub fn compile_func(&mut self, loc: &Value, func: &Function) -> CFunc {
        let mut blocks = Vec::with_capacity(func.basic_blocks.len());
        for (i, block) in func.basic_blocks.iter().enumerate() {
            self.block_ids.insert(block.name.clone(), CBlockId(i));
        }
        for (i, block) in func.basic_blocks.iter().enumerate() {
            blocks.push(self.compile_block(loc, CBlockId(i), block));
        }
        CFunc {
            fp: loc.as_u64(),
            src: func.clone(),
            args: func.parameters.iter().map(|p| self.compile_local(&p.name)).collect(),
            blocks,
            locals: self.local_names.clone(),
        }
    }
}

