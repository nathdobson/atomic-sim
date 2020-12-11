use llvm_ir::{Function, Module, Name, Instruction, Terminator, Operand, BasicBlock, ConstantRef, Constant, IntPredicate};
use std::rc::Rc;
use llvm_ir::instruction::*;
use crate::symbols::{Symbol, SymbolTable, ModuleId};
use crate::value::{Value, add_u64_i64};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use crate::class::{TypeMap, Class, ClassKind, VectorClass};
use crate::layout::{Layout, Packing};
use crate::memory::Memory;
use crate::process::Process;
use crate::thread::ThreadId;
use llvm_ir::constant;
use llvm_ir::terminator::*;
use either::Either;
use crate::arith;
use itertools::Itertools;
use crate::operation::{COperationName, COperation, CFPPredicate};
use crate::operation::CIntPredicate;
use llvm_ir::constant::Float;
use llvm_ir::types::FPType;

#[derive(Debug)]
pub struct CFunc {
    pub src: Function,
    pub blocks: Vec<CBlock>,
    pub locals: Vec<Name>,
}

#[derive(Debug)]
pub struct CBlock {
    phis: Vec<CPhi>,
    instrs: Vec<CInstr>,
    term: CTerm,
}

#[derive(Debug)]
pub struct CLoad {
    dest: CLocal,
    address: COperand,
}

#[derive(Debug)]
pub struct CStore {
    address: COperand,
    value: COperand,
}


#[derive(Debug)]
pub struct CCompute {
    dest: CLocal,
    operands: Vec<COperand>,
    operation: COperation,
}

#[derive(Debug)]
pub struct CSelect {
    dest: CLocal,
    condition: COperand,
    true_value: COperand,
    false_value: COperand,
}

#[derive(Debug)]
pub struct CAlloca {
    dest: CLocal,
    count: COperand,
    layout: Layout,
}

#[derive(Debug)]
pub struct CCall {}

#[derive(Debug)]
pub enum CInstr {
    Compute(CCompute),

    Select(CSelect),
    Call(CCall),
    Alloca(CAlloca),

    Load(CLoad),
    Store(CStore),

    Fence,
    CmpXchg,
    AtomicRMW,
    Freeze,

    LandingPad,
}

#[derive(Debug)]
pub struct CRet {
    return_operand: COperand,
}

#[derive(Debug)]
pub struct CBr {
    target: CBlockId,
}

#[derive(Debug)]
pub struct CCondBr {
    condition: COperand,
    true_target: CBlockId,
    false_target: CBlockId,
}

#[derive(Debug)]
pub struct CSwitch {
    condition: COperand,
    targets: HashMap<Value, CBlockId>,
    default_target: CBlockId,
}

#[derive(Debug)]
pub struct CInvoke {
    function: COperand,
    arguments: Vec<COperand>,
    dest: CLocal,
    target: CBlockId,
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
    Constant(Class, Value),
    Local(Class, CLocal),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CBlockId(pub usize);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CLocal(pub usize);

#[derive(Debug)]
pub struct CPhi {
    dest: CLocal,
    mapping: HashMap<CBlockId, COperand>,
}


struct FuncCompiler {
    locals: HashMap<Name, CLocal>,
    local_names: Vec<Name>,
    block_ids: HashMap<Name, CBlockId>,
    type_map: TypeMap,
    module_compiler: Rc<ModuleCompiler>,
}

impl FuncCompiler {
    fn compile_local(&mut self, name: &Name) -> CLocal {
        let local_names = &mut self.local_names;
        *self.locals.entry(name.clone()).or_insert_with(|| {
            let local = CLocal(local_names.len());
            local_names.push(name.clone());
            local
        })
    }
    fn compile_const(&mut self, constant: &ConstantRef) -> (Class, Value) {
        self.module_compiler.compile_const(constant)
    }
    fn compile_operand(&mut self, operand: &Operand) -> COperand {
        match operand {
            Operand::LocalOperand { name, ty } =>
                COperand::Local(self.type_map.get(ty), self.compile_local(name)),
            Operand::ConstantOperand(expr) => {
                let (class, value) = self.compile_const(expr);
                COperand::Constant(class, value)
            }
            Operand::MetadataOperand =>
                COperand::Constant(self.type_map.void(), Value::from(())),
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
        let operation = self.module_compiler.compile_operation(&operand_classes, name);
        CInstr::Compute(CCompute {
            dest: self.compile_local(dest),
            operands,
            operation,
        })
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
                CInstr::Compute(CCompute {
                    dest: self.compile_local(dest),
                    operands: vec![operand0, operand1],
                    operation: COperation {
                        input: vec![class.clone(), class.clone()],
                        output: class,
                        name: self.compile_binop(instr),
                    },
                })
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
                            output = output.element(c.as_i64()).class,
                        COperand::Local(_, _) =>
                            output = output.element(-1).class,
                    }
                    input.push(operand.class().clone());
                    operands.push(operand);
                }
                CInstr::Compute(CCompute {
                    dest: self.compile_local(dest),
                    operands,
                    operation: COperation {
                        input,
                        output: self.type_map.ptr(output),
                        name: COperationName::GetElementPtr,
                    },
                })
            }
            Instruction::ExtractElement(ExtractElement { vector, index, dest, debugloc }) => {
                self.compile_compute(dest, &[vector, index], COperationName::ExtractElement)
            }
            Instruction::InsertElement(InsertElement { vector, element, index, dest, debugloc }) => {
                self.compile_compute(dest, &[vector, element, index], COperationName::InsertElement)
            }
            Instruction::ShuffleVector(ShuffleVector { operand0, operand1, dest, mask, debugloc }) => {
                let (mask_class, mask) = self.module_compiler.compile_const(mask);
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
                self.compile_compute(dest, &[aggregate, element], COperationName::InsertValue)
            }
            Instruction::Select(Select { condition, true_value, false_value, dest, debugloc }) => {
                CInstr::Select(CSelect {
                    dest: self.compile_local(dest),
                    condition: self.compile_operand(condition),
                    true_value: self.compile_operand(true_value),
                    false_value: self.compile_operand(false_value),
                })
            }
            Instruction::Phi(Phi { incoming_values, dest, to_type, debugloc }) => {
                unreachable!()
            }
            Instruction::Alloca(Alloca { allocated_type, num_elements, dest, alignment, debugloc }) => {
                CInstr::Alloca(CAlloca {
                    dest: self.compile_local(dest),
                    count: self.compile_operand(num_elements),
                    layout: self.type_map.get(allocated_type).layout().align_to_bits((8 * *alignment) as u64),
                })
            }
            Instruction::Load(Load { address, dest, volatile, atomicity, alignment, debugloc }) => {
                CInstr::Load(CLoad {
                    dest: self.compile_local(dest),
                    address: self.compile_operand(address),
                })
            }
            Instruction::Store(Store { address, value, volatile, atomicity, alignment, debugloc }) => {
                CInstr::Store(CStore {
                    address: self.compile_operand(address),
                    value: self.compile_operand(value),
                })
            }
            Instruction::Fence(Fence { atomicity, debugloc }) => {
                CInstr::Fence
            }
            Instruction::CmpXchg(CmpXchg { address, expected, replacement, dest, volatile, atomicity, failure_memory_ordering, debugloc, weak }) => {
                CInstr::CmpXchg
            }
            Instruction::AtomicRMW(AtomicRMW { address, value, dest, operation, volatile, atomicity, debugloc }) => {
                CInstr::AtomicRMW
            }
            Instruction::Freeze(Freeze { operand, dest, debugloc }) => {
                CInstr::Freeze
            }
            Instruction::Call(Call {
                                  function,
                                  arguments, return_attributes, dest, function_attributes, is_tail_call, calling_convention, debugloc
                              }) => {
                CInstr::Call(CCall {})
            }
            Instruction::VAArg(_) => {
                todo!()
            }
            Instruction::LandingPad(_) => {
                CInstr::LandingPad
            }
            Instruction::CatchPad(_) => {
                todo!()
            }
            Instruction::CleanupPad(_) => {
                todo!()
            }
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
                    COperand::Constant(self.type_map.void(), Value::from(()))
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
                        (self.compile_const(pat).1, self.compile_target(target))
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
                    function: match function {
                        Either::Left(_) => todo!(),
                        Either::Right(function) => self.compile_operand(function),
                    },
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
    fn compile_block(&mut self, block: &BasicBlock) -> CBlock {
        let mut phis = vec![];
        let mut instrs = vec![];
        for instr in block.instrs.iter() {
            match instr {
                Instruction::Phi(phi) => phis.push(self.compile_phi(&phi)),
                instr => instrs.push(self.compile_instr(&instr)),
            }
        }
        let term = self.compile_term(&block.term);
        CBlock { phis, instrs, term }
    }
    fn compile_func(&mut self, func: &Function) -> CFunc {
        let mut blocks = Vec::with_capacity(func.basic_blocks.len());
        for (i, block) in func.basic_blocks.iter().enumerate() {
            self.block_ids.insert(block.name.clone(), CBlockId(i));
        }
        for block in func.basic_blocks.iter() {
            blocks.push(self.compile_block(block));
        }
        CFunc {
            src: func.clone(),
            blocks,
            locals: self.local_names.clone(),
        }
    }
}

struct ModuleCompiler {
    moduleid: ModuleId,
    src: Rc<Module>,
    process: Process,
    type_map: TypeMap,
}

pub struct Compiler {
    ptr_bits: u64,
    process: Process,
}

impl ModuleCompiler {
    fn compile_operation(&self, operands: &[&Class], name: COperationName) -> COperation {
        use COperationName::*;
        let output = match &name {
            Add
            | Sub
            | Mul
            | UDiv
            | SDiv
            | URem
            | SRem
            | And
            | Or
            | Xor
            | Shl
            | LShr
            | AShr
            | FAdd
            | FSub
            | FMul
            | FDiv
            | FRem
            | FNeg => {
                assert!(operands.iter().all_equal());
                operands[0].clone()
            }
            ZExt(target)
            | SExt(target)
            | Trunc(target)
            | FPTrunc(target)
            | FPExt(target)
            | FPToUI(target)
            | FPToSI(target)
            | UIToFP(target)
            | SIToFP(target) => {
                match operands[0].kind() {
                    ClassKind::VectorClass(VectorClass { element, len }) =>
                        self.type_map.vector(target.clone(), *len),
                    _ => target.clone()
                }
            }
            ICmp(_)
            | FCmp(_) => {
                match operands[0].kind() {
                    ClassKind::VectorClass(VectorClass { element, len }) =>
                        self.type_map.vector(self.type_map.bool(), *len),
                    _ => self.type_map.bool()
                }
            }
            BitCast(to) => to.clone(),
            GetElementPtr => todo!(),
            ExtractElement => {
                operands[0].element(-1).class
            }
            ExtractValue(indices) => operands[0].element_rec(&mut indices.iter().cloned()).class.clone(),
            InsertElement => operands[0].clone(),
            InsertValue => operands[0].clone(),
            Shuffle(indices) => {
                let l = operands[0].as_vector().unwrap();
                self.type_map.vector(l.element.clone(), indices.len() as u64)
            }
        };
        COperation {
            input: operands.iter().cloned().cloned().collect(),
            output,
            name,
        }
    }

    fn compile_cast(&self, constant: &ConstantRef, target: Class) -> COperationName {
        use COperationName::*;
        match &**constant {
            Constant::Trunc(_) => Trunc(target),
            Constant::ZExt(_) => ZExt(target),
            Constant::SExt(_) => SExt(target),
            Constant::FPTrunc(_) => FPTrunc(target),
            Constant::FPExt(_) => FPExt(target),
            Constant::FPToUI(_) => FPToUI(target),
            Constant::FPToSI(_) => FPToSI(target),
            Constant::UIToFP(_) => UIToFP(target),
            Constant::SIToFP(_) => SIToFP(target),
            Constant::PtrToInt(_) => ZExt(target),
            Constant::IntToPtr(_) => ZExt(target),
            Constant::BitCast(_) => BitCast(target),
            Constant::AddrSpaceCast(_) => ZExt(target),
            _ => unreachable!(),
        }
    }

    fn compile_binop(&self, constant: &ConstantRef) -> COperationName {
        use COperationName::*;
        match &**constant {
            Constant::Add(_) => Add,
            Constant::Sub(_) => Sub,
            Constant::Mul(_) => Mul,
            Constant::UDiv(_) => UDiv,
            Constant::SDiv(_) => SDiv,
            Constant::URem(_) => URem,
            Constant::SRem(_) => SRem,
            Constant::And(_) => And,
            Constant::Or(_) => Or,
            Constant::Xor(_) => Xor,
            Constant::Shl(_) => Shl,
            Constant::LShr(_) => LShr,
            Constant::AShr(_) => AShr,
            Constant::FAdd(_) => FAdd,
            Constant::FSub(_) => FSub,
            Constant::FMul(_) => FMul,
            Constant::FDiv(_) => FDiv,
            Constant::FRem(_) => FRem,
            _ => unreachable!(),
        }
    }

    fn compile_const(&self, constant: &ConstantRef) -> (Class, Value) {
        match &**constant {
            Constant::Int { bits, value } => {
                (self.type_map.int(*bits as u64), Value::new(*bits as u64, *value as u128))
            }
            Constant::BitCast(constant::BitCast { operand, to_type })
            | Constant::IntToPtr(constant::IntToPtr { operand, to_type })
            | Constant::Trunc(constant::Trunc { operand, to_type })
            | Constant::SExt(constant::SExt { operand, to_type })
            | Constant::ZExt(constant::ZExt { operand, to_type })
            | Constant::PtrToInt(constant::PtrToInt { operand, to_type }) => {
                let to_class = self.type_map.get(to_type);
                let (class, value) = self.compile_const(&operand);
                let operator = self.compile_operation(&[&class],
                                                      self.compile_cast(constant, to_class.clone()));
                (to_class, value)
            }
            Constant::GlobalReference { name, ty } => {
                let name = match name {
                    Name::Name(name) => name,
                    Name::Number(_) => panic!(),
                };
                (self.type_map.ptr(self.type_map.get(ty)), self.process.lookup(Some(self.moduleid), name).clone())
            }
            Constant::Undef(typ) => {
                (self.type_map.get(typ), Value::zero(self.type_map.get(typ).layout()))
            }
            Constant::Null(typ) => {
                (self.type_map.get(typ), Value::zero(self.type_map.get(typ).layout()))
            }
            Constant::AggregateZero(typ) => {
                (self.type_map.get(typ), Value::zero(self.type_map.get(typ).layout()))
            }
            Constant::Struct { name, values, is_packed, } => {
                let children = values.iter().map(|c| self.compile_const(c)).collect::<Vec<_>>();
                let class = if let Some(name) = name {
                    self.type_map.get(&self.src.types.named_struct(name).unwrap())
                } else {
                    self.type_map.struc(children.iter().map(|(ty, v)| ty.clone()).collect(),
                                        *is_packed)
                };
                let value = Value::aggregate(&class, children.into_iter().map(|(ty, v)| v));
                (class, value)
            }
            Constant::Array { element_type, elements } => {
                let class = self.type_map.array(self.type_map.get(element_type), elements.len() as u64);
                let value = Value::aggregate(&class, elements.iter().map(|c| {
                    let (c, v) = self.compile_const(c);
                    assert_eq!(c, self.type_map.get(element_type));
                    v
                }));
                (class, value)
            }
            Constant::GetElementPtr(constant::GetElementPtr { address, indices, in_bounds }) => {
                let (c1, address) = self.compile_const(address);
                let mut indices = indices.iter().map(|c| self.compile_const(c).1.as_i64());
                let element = c1.element_rec(&mut indices);
                assert_eq!(element.bit_offset % 8, 0);
                (self.type_map.ptr(element.class),
                 self.process.value_from_address(add_u64_i64(address.as_u64(), element.bit_offset / 8)))
            }
            Constant::Vector(vec) => {
                let elems = vec.iter().map(|c| self.compile_const(c)).collect::<Vec<_>>();
                let class = self.type_map.vector(elems[0].0.clone(), vec.len() as u64);
                let value = Value::aggregate(&class, elems.into_iter().map(|(t, v)| v));
                (class, value)
            }
            Constant::Select(constant::Select { condition, true_value, false_value }) => {
                if self.compile_const(condition).1.unwrap_bool() {
                    self.compile_const(true_value)
                } else {
                    self.compile_const(false_value)
                }
            }
            Constant::Float(f) => {
                match f {
                    Float::Half => todo!(),
                    Float::Single(f) => (self.type_map.float(FPType::Single), Value::from(f.to_bits())),
                    Float::Double(f) => (self.type_map.float(FPType::Double), Value::from(f.to_bits())),
                    Float::Quadruple => todo!(),
                    Float::X86_FP80 => todo!(),
                    Float::PPC_FP128 => todo!(),
                }
            }
            Constant::Add(constant::Add { operand0, operand1 })
            | Constant::Sub(constant::Sub { operand0, operand1 })
            | Constant::Mul(constant::Mul { operand0, operand1 })
            | Constant::UDiv(constant::UDiv { operand0, operand1 })
            | Constant::SDiv(constant::SDiv { operand0, operand1 })
            | Constant::URem(constant::URem { operand0, operand1 })
            | Constant::SRem(constant::SRem { operand0, operand1 })
            | Constant::And(constant::And { operand0, operand1 })
            | Constant::Or(constant::Or { operand0, operand1 })
            | Constant::Xor(constant::Xor { operand0, operand1 })
            | Constant::Shl(constant::Shl { operand0, operand1 })
            | Constant::LShr(constant::LShr { operand0, operand1 })
            | Constant::AShr(constant::AShr { operand0, operand1 })
            | Constant::FAdd(constant::FAdd { operand0, operand1 })
            | Constant::FSub(constant::FSub { operand0, operand1 })
            | Constant::FMul(constant::FMul { operand0, operand1 })
            | Constant::FDiv(constant::FDiv { operand0, operand1 })
            | Constant::FRem(constant::FRem { operand0, operand1 }) => {
                let operand0 = self.compile_const(operand0);
                let operand1 = self.compile_const(operand1);
                let oper = self.compile_operation(&[&operand0.0, &operand1.0], self.compile_binop(&constant));
                (oper.output.clone(), oper.call_operation(&[&operand0.1, &operand1.1]))
            }
            Constant::ICmp(constant::ICmp { predicate, operand0, operand1 }) => {
                let operand0 = self.compile_const(operand0);
                let operand1 = self.compile_const(operand1);
                let oper = self.compile_operation(&[&operand0.0, &operand1.0],
                                                  COperationName::ICmp(CIntPredicate::from(*predicate)));
                (oper.output.clone(), oper.call_operation(&[&operand0.1, &operand1.1]))
            }
            Constant::FCmp(constant::FCmp { predicate, operand0, operand1 }) => {
                let operand0 = self.compile_const(operand0);
                let operand1 = self.compile_const(operand1);
                let oper = self.compile_operation(&[&operand0.0, &operand1.0],
                                                  COperationName::FCmp(CFPPredicate::from(*predicate)));
                (oper.output.clone(), oper.call_operation(&[&operand0.1, &operand1.1]))
            }
            Constant::ExtractElement(_) => todo!(),
            Constant::InsertElement(_) => todo!(),
            Constant::ShuffleVector(_) => todo!(),
            Constant::ExtractValue(_) => todo!(),
            Constant::InsertValue(_) => todo!(),
            Constant::FPTrunc(_) => todo!(),
            Constant::FPExt(_) => todo!(),
            Constant::FPToUI(_) => todo!(),
            Constant::FPToSI(_) => todo!(),
            Constant::UIToFP(_) => todo!(),
            Constant::SIToFP(_) => todo!(),
            Constant::AddrSpaceCast(_) => todo!(),
            Constant::BlockAddress => todo!(),
            Constant::TokenNone => todo!(),
        }
    }
    fn const_operation(&self, operands: &[&ConstantRef], oper: COperationName) -> (Class, Value) {
        let inputs =
            operands.iter()
                .map(|operand| self.compile_const(operand))
                .collect::<Vec<_>>();
        let input_classes = inputs.iter().map(|operand| &operand.0).collect::<Vec<_>>();
        let input_values = inputs.iter().map(|operand| &operand.1).collect::<Vec<_>>();
        let oper = self.compile_operation(&input_classes, oper);
        let output = oper.call_operation(&input_values);
        (oper.output, output)
    }
    fn map_vector(&self, c: &Class, f: impl FnOnce(&Class) -> Class) -> Class {
        match c.kind() {
            ClassKind::VectorClass(VectorClass { element, len }) => {
                self.type_map.vector(f(element), *len)
            }
            _ => f(c)
        }
    }
}

impl Compiler {
    pub fn new(ptr_bits: u64, process: Process) -> Self {
        Compiler {
            ptr_bits,
            process,
        }
    }
    pub fn compile_modules(&mut self, modules: Vec<Module>) -> HashMap<u64, CFunc> {
        let mut funcs = HashMap::new();
        let type_map = TypeMap::new(self.ptr_bits);
        for module in modules.iter() {
            type_map.add_module(module);
        }

        let modules = modules.into_iter().enumerate().map(|(index, module)| {
            let module = Rc::new(module);
            Rc::new(ModuleCompiler {
                moduleid: ModuleId(index),
                src: module.clone(),
                process: self.process.clone(),
                type_map: type_map.clone(),
            })
        }).collect::<Vec<_>>();
        let func_layout = Layout::of_func();
        let mut func_inits = vec![];
        let mut global_inits = vec![];
        for module in modules.iter() {
            for func in module.src.functions.iter() {
                let symbol = Symbol::new(func.linkage, module.moduleid, &func.name);
                let loc = self.process.add_symbol(symbol.clone(), func_layout);
                func_inits.push((module, loc, func));
            }
            for g in module.src.global_vars.iter() {
                let name = str_of_name(&g.name);
                let symbol = Symbol::new(g.linkage, module.moduleid, name);
                let mut layout = module.type_map.get(&g.ty).element(0).class.layout();
                layout = layout.align_to_bits(8 * g.alignment as u64);
                let loc = self.process.add_symbol(symbol.clone(), layout);
                global_inits.push((module, loc, layout, g));
            }
        }
        for module in modules.iter() {
            for g in module.src.global_aliases.iter() {
                let name = str_of_name(&g.name);
                let symbol = Symbol::new(g.linkage, module.moduleid, name);
                let (_, target) = module.compile_const(&g.aliasee);
                self.process.add_alias(symbol, target);
            }
        }
        for (module, loc, func) in func_inits {
            funcs.insert(loc.as_u64(), FuncCompiler {
                locals: HashMap::new(),
                local_names: vec![],
                block_ids: HashMap::new(),
                type_map: module.type_map.clone(),
                module_compiler: module.clone(),
            }.compile_func(func));
        }
        for (module, loc, layout, global) in global_inits {
            let value = if let Some(init) = &global.initializer {
                let (c, value) = module.compile_const(&init);
                assert_eq!(c.layout().bits(), layout.bits());
                assert!(c.layout().bit_align() <= layout.bit_align());
                value
            } else {
                Value::zero(layout)
            };
            self.process.store(ThreadId(0), &loc, &value, None);
        }
        funcs
    }
}

impl COperand {
    fn class(&self) -> &Class {
        match self {
            COperand::Constant(c, _) => c,
            COperand::Local(c, _) => c,
        }
    }
}

fn str_of_name(name: &Name) -> &str {
    match name {
        Name::Name(name) => &***name,
        Name::Number(_) => panic!(),
    }
}
