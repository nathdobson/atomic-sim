use crate::compile::class::{Class, TypeMap};
use crate::symbols::{SymbolDef, ThreadLocalKey};
use crate::value::Value;
use crate::compile::operation::{COperation, COperationName, OperCompiler};
use llvm_ir::{Constant, ConstantRef, Name};
use llvm_ir::constant;
use crate::process::Process;
use llvm_ir::constant::Float;
use llvm_ir::types::FPType;
use std::rc::Rc;
use crate::compile::operation::CIntPredicate;
use crate::compile::operation::CFPPredicate;
use crate::compile::module::{ModuleCompiler, ModuleId};
use std::iter;

#[derive(Debug)]
pub enum CExpr {
    Const { class: Class, value: Value },
    ThreadLocal { class: Class, key: ThreadLocalKey },
    Apply { oper: COperation, args: Vec<CExpr> },
}

pub struct ExprCompiler {
    moduleid: ModuleId,
    process: Process,
    oper_compiler: Rc<OperCompiler>,
}

impl CExpr {
    pub fn class(&self) -> &Class {
        match self {
            CExpr::Const { class, value } => class,
            CExpr::Apply { oper, args } => &oper.output,
            CExpr::ThreadLocal { class, key } => class
        }
    }
    pub fn as_const(&self) -> Option<(&Class, &Value)> {
        match self {
            CExpr::Const { class, value } => Some((class, value)),
            _ => None,
        }
    }
    pub fn reduce_root(&mut self) {
        if let Some((class, value)) = match self {
            CExpr::Apply { oper, args } => {
                args.iter_mut()
                    .map(|arg| arg.as_const().map(|(c, v)| v))
                    .collect::<Option<Vec<&Value>>>()
                    .map(|args| (oper.output.clone(), oper.call_operation(&args)))
            }
            _ => None
        } {
            *self = CExpr::Const { class, value };
        }
    }
}

impl ExprCompiler {
    pub fn new(moduleid: ModuleId,
               process: Process,
               oper_compiler: Rc<OperCompiler>) -> Self {
        ExprCompiler { moduleid, process, oper_compiler }
    }
    fn apply(&self, oper: COperationName, args: Vec<CExpr>) -> CExpr {
        CExpr::Apply {
            oper: self.oper_compiler.compile_operation(
                oper,
                &args.iter().map(|e| e.class()).collect::<Vec<_>>(),
            ),
            args,
        }
    }
    pub fn type_map(&self) -> TypeMap {
        self.process.types.clone()
    }
    pub fn compile_oper(&self, oper: COperationName, input: &[&Class]) -> COperation {
        todo!()
    }
    pub fn compile_const(&self, constant: &ConstantRef) -> CExpr {
        let mut result = self.compile_const_impl(constant);
        result.reduce_root();
        result
    }
    pub fn compile_const_impl(&self, constant: &ConstantRef) -> CExpr {
        match &**constant {
            Constant::Int { bits, value } =>
                CExpr::Const {
                    class: self.type_map().int(*bits as u64),
                    value: Value::new(*bits as u64, *value as u128),
                },
            Constant::BitCast(constant::BitCast { operand, to_type })
            | Constant::IntToPtr(constant::IntToPtr { operand, to_type })
            | Constant::Trunc(constant::Trunc { operand, to_type })
            | Constant::SExt(constant::SExt { operand, to_type })
            | Constant::ZExt(constant::ZExt { operand, to_type })
            | Constant::PtrToInt(constant::PtrToInt { operand, to_type }) => {
                let to_class = self.type_map().get(to_type);
                let inner = self.compile_const(&operand);
                self.apply(self.compile_cast(constant, to_class.clone()), vec![inner])
            }
            Constant::GlobalReference { name, ty } => {
                let name = match name {
                    Name::Name(name) => name,
                    Name::Number(_) => panic!(),
                };
                let def = self.process.symbols.lookup(Some(self.moduleid), name);
                match def {
                    SymbolDef::Global(g) =>
                        CExpr::Const {
                            class: self.type_map().ptr(self.type_map().get(ty)),
                            value: self.process.addr(g),
                        },
                    SymbolDef::ThreadLocal(key) =>
                        CExpr::ThreadLocal {
                            class: self.type_map().ptr(self.type_map().get(ty)),
                            key,
                        },
                }
            }
            Constant::Undef(typ) => {
                CExpr::Const {
                    class: self.type_map().get(typ),
                    value: Value::zero(self.type_map().get(typ).layout().bits()),
                }
            }
            Constant::Null(typ) => {
                CExpr::Const {
                    class: self.type_map().get(typ),
                    value: Value::zero(self.type_map().get(typ).layout().bits()),
                }
            }
            Constant::AggregateZero(typ) => {
                CExpr::Const {
                    class: self.type_map().get(typ),
                    value: Value::zero(self.type_map().get(typ).layout().bits()),
                }
            }
            Constant::Struct { name, values, is_packed, } => {
                let children = values.iter().map(|c| self.compile_const(c)).collect::<Vec<_>>();
                let class = if let Some(name) = name {
                    self.type_map().named_struct(&name).unwrap()
                } else {
                    self.type_map().struc(
                        children.iter()
                            .map(|c| c.class().clone()).collect(),
                        *is_packed)
                };
                self.apply(COperationName::Aggregate(class.clone()), children)
            }
            Constant::Array { element_type, elements } => {
                let children = elements.iter().map(|c| self.compile_const(c)).collect::<Vec<_>>();
                let class = self.type_map().array(self.type_map().get(element_type), elements.len() as u64);
                self.apply(COperationName::Aggregate(class.clone()), children)
            }
            Constant::Vector(elements) => {
                let children = elements.iter().map(|c| self.compile_const(c)).collect::<Vec<_>>();
                let class = self.type_map().vector(children[0].class().clone(), elements.len() as u64);
                self.apply(COperationName::Aggregate(class.clone()), children)
            }
            Constant::GetElementPtr(constant::GetElementPtr { address, indices, in_bounds }) => {
                let address = self.compile_const(address);
                let args =
                    iter::once(address)
                        .chain(
                            indices.iter()
                                .map(|c| self.compile_const(c))
                        ).collect::<Vec<_>>();
                let element = args[0].class().element_rec(&mut args[1..].iter().map(|c| {
                    c.as_const().map_or(-1, |(c, v)| v.as_i64())
                }));
                assert_eq!(element.bit_offset % 8, 0);
                CExpr::Apply {
                    oper: COperation {
                        input: args.iter().map(|x| x.class().clone()).collect(),
                        output: self.type_map().ptr(element.class),
                        name: COperationName::GetElementPtr,
                    },
                    args,
                }
            }
            Constant::Select(constant::Select { condition, true_value, false_value }) => {
                let condition = self.compile_const(condition);
                let true_value = self.compile_const(true_value);
                let false_value = self.compile_const(false_value);
                self.apply(COperationName::Select, vec![condition, true_value, false_value])
            }
            Constant::Float(f) => {
                match f {
                    Float::Half => todo!(),
                    Float::Single(f) =>
                        CExpr::Const {
                            class: self.type_map().float(FPType::Single),
                            value: Value::from(f.to_bits()),
                        },
                    Float::Double(f) =>
                        CExpr::Const {
                            class: self.type_map().float(FPType::Double),
                            value: Value::from(f.to_bits()),
                        },
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
                self.apply(self.compile_binop(&constant), vec![operand0, operand1])
            }
            Constant::ICmp(constant::ICmp { predicate, operand0, operand1 }) => {
                let operand0 = self.compile_const(operand0);
                let operand1 = self.compile_const(operand1);
                self.apply(COperationName::ICmp(CIntPredicate::from(*predicate)), vec![operand0, operand1])
            }
            Constant::FCmp(constant::FCmp { predicate, operand0, operand1 }) => {
                let operand0 = self.compile_const(operand0);
                let operand1 = self.compile_const(operand1);
                self.apply(COperationName::FCmp(CFPPredicate::from(*predicate)), vec![operand0, operand1])
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
}