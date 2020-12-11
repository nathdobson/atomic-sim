use llvm_ir::{TypeRef, Type, IntPredicate, FPPredicate};
use crate::value::Value;
use std::ops::Add;
use std::iter;
use crate::layout::{Layout, Packing};
use crate::class::{Class, ClassKind};
use bitvec::vec::BitVec;

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum CIntPredicate {
    EQ,
    NE,
    UGT,
    UGE,
    ULT,
    ULE,
    SGT,
    SGE,
    SLT,
    SLE,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum CFPPredicate {
    False,
    OEQ,
    OGT,
    OGE,
    OLT,
    OLE,
    ONE,
    ORD,
    UNO,
    UEQ,
    UGT,
    UGE,
    ULT,
    ULE,
    UNE,
    True,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum COperationName {
    Add,
    Sub,
    Mul,
    UDiv,
    SDiv,
    URem,
    SRem,
    And,
    Or,
    Xor,
    Shl,
    LShr,
    AShr,
    FAdd,
    FSub,
    FMul,
    FDiv,
    FRem,
    FNeg,
    ICmp(CIntPredicate),
    FCmp(CFPPredicate),
    GetElementPtr,
    ExtractElement,
    ExtractValue(Vec<i64>),
    InsertElement,
    InsertValue,
    Shuffle(Vec<i64>),
    Trunc(Class),
    ZExt(Class),
    SExt(Class),
    FPTrunc(Class),
    FPExt(Class),
    FPToUI(Class),
    FPToSI(Class),
    UIToFP(Class),
    SIToFP(Class),
    BitCast(Class),
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct COperation {
    pub input: Vec<Class>,
    pub output: Class,
    pub name: COperationName,
}

macro_rules! repeat5 {
    ($x:expr)=>(($x,$x,$x,$x,$x))
}
impl COperation {
    pub fn call_operation(&self, inputs: &[&Value]) -> Value {
        use COperationName::*;
        match &self.name {
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
            | FNeg
            | ICmp(_)
            | FCmp(_)
            | SExt(_)
            | ZExt(_)
            | Trunc(_)
            | FPTrunc(_)
            | FPExt(_)
            | FPToUI(_)
            | FPToSI(_)
            | UIToFP(_)
            | SIToFP(_)
            => {
                match &self.input[0].kind() {
                    ClassKind::IntegerClass(_)
                    | ClassKind::PointerClass(_)
                    | ClassKind::FPClass(_) => self.call_scalar(inputs),
                    ClassKind::VectorClass(_) => todo!(),
                    _ => todo!(),
                }
            }
            BitCast(_) => todo!(),
            GetElementPtr => todo!(),
            ExtractElement => todo!(),
            ExtractValue(indices) => {
                todo!()
            }
            InsertElement => todo!(),
            InsertValue => todo!(),
            Shuffle(indices) => todo!(),
        }
    }

    pub fn call_scalar(&self, inputs: &[&Value]) -> Value {
        use COperationName::*;
        match &self.name {
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
            | FNeg
            | ICmp(_) =>
                self.call_scalar_binop(inputs[0], inputs[1]),
            | SExt(_) => self.call_scalar_sext(inputs[0]),
            | ZExt(_) => inputs[0].ucast(self.output.layout()),
            _ => todo!()
        }
    }

    fn call_scalar_binop(&self, v1: &Value, v2: &Value) -> Value {
        use COperationName::*;
        match &self.name {
            Add => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_add(y))),
            Sub => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_sub(y))),
            Mul => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_mul(y))),
            UDiv => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_div(y))),
            SDiv => call_scalar_binop_signed(v1, v2, repeat5!(|x,y| x.wrapping_div(y))),
            URem => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_rem(y))),
            SRem => call_scalar_binop_signed(v1, v2, repeat5!(|x,y| x.wrapping_rem(y))),
            Xor => call_scalar_binop_unsigned(v1, v2, |x, y| x ^ y, repeat5!(|x,y| x^y)),
            And => call_scalar_binop_unsigned(v1, v2, |x, y| x & y, repeat5!(|x,y| x&y)),
            Or => call_scalar_binop_unsigned(v1, v2, |x, y| x | y, repeat5!(|x,y| x|y)),
            Shl => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x<<y)),
            LShr => call_scalar_lshr(v1, v2),
            AShr => call_scalar_binop_signed(v1, v2, repeat5!(|x,y| x>>y)),
            FAdd => todo!(),
            FSub => todo!(),
            FMul => todo!(),
            FDiv => todo!(),
            FRem => todo!(),
            FNeg => todo!(),
            ICmp(predicate) => {
                match predicate {
                    CIntPredicate::EQ => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x==y)),
                    CIntPredicate::NE => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x!=y)),
                    CIntPredicate::UGT => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x>y)),
                    CIntPredicate::UGE => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x>=y)),
                    CIntPredicate::ULT => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x<y)),
                    CIntPredicate::ULE => call_scalar_binop_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x<=y)),
                    CIntPredicate::SGT => call_scalar_binop_signed(v1, v2, repeat5!(|x,y| x>y)),
                    CIntPredicate::SGE => call_scalar_binop_signed(v1, v2, repeat5!(|x,y| x>=y)),
                    CIntPredicate::SLT => call_scalar_binop_signed(v1, v2, repeat5!(|x,y| x<y)),
                    CIntPredicate::SLE => call_scalar_binop_signed(v1, v2, repeat5!(|x,y| x<=y)),
                }
            }
            _ => unreachable!("{:?}", self)
        }
    }

    pub fn call_scalar_sext(&self, v1: &Value) -> Value {
        match (v1.bits(), self.output.layout().bits()) {
            (x, y) if x == y => v1.clone(),
            (8, 16) => Value::from(v1.unwrap_i8() as i16),
            (8, 32) => Value::from(v1.unwrap_i8() as i32),
            (8, 64) => Value::from(v1.unwrap_i8() as i64),
            (16, 32) => Value::from(v1.unwrap_i16() as i32),
            (16, 64) => Value::from(v1.unwrap_i16() as i64),
            (32, 64) => Value::from(v1.unwrap_i32() as i64),
            bits => todo!("{:?}", bits),
        }
    }

}

pub fn call_scalar_lshr(v1: &Value, v2: &Value) -> Value {
    let width = v1.bits() as usize;
    let shift = v2.as_u64() as usize;
    let mut output = BitVec::repeat(false, width);
    output[..width - shift].copy_from_bitslice(&v1.as_bits()[shift..]);
    Value::from_bits(v1.layout(), output)
}

fn call_scalar_binop_unsigned<T1, T8, T16, T32, T64, T128>
(v1: &Value, v2: &Value,
 f1: fn(bool, bool) -> T1,
 (f8, f16, f32, f64, f128):
 (fn(u8, u8) -> T8,
  fn(u16, u16) -> T16,
  fn(u32, u32) -> T32,
  fn(u64, u64) -> T64,
  fn(u128, u128) -> T128),
) -> Value where Value: From<T1> + From<T8> + From<T16> + From<T32> + From<T64> + From<T128> {
    match (v1.bits(), v2.bits()) {
        (1, 1) => Value::from(f1(v1.unwrap_bool(), v2.unwrap_bool())),
        (8, 8) => Value::from(f8(v1.unwrap_u8(), v2.unwrap_u8())),
        (16, 16) => Value::from(f16(v1.unwrap_u16(), v2.unwrap_u16())),
        (32, 32) => Value::from(f32(v1.unwrap_u32(), v2.unwrap_u32())),
        (64, 64) => Value::from(f64(v1.unwrap_u64(), v2.unwrap_u64())),
        (128, 128) => Value::from(f128(v1.unwrap_u128(), v2.unwrap_u128())),
        bits => todo!("{:?}", bits)
    }
}

fn call_scalar_binop_signed<T8, T16, T32, T64, T128>
(v1: &Value, v2: &Value,
 (f8, f16, f32, f64, f128):
 (fn(i8, i8) -> T8,
  fn(i16, i16) -> T16,
  fn(i32, i32) -> T32,
  fn(i64, i64) -> T64,
  fn(i128, i128) -> T128),
) -> Value where Value: From<T8> + From<T16> + From<T32> + From<T64> + From<T128> {
    match (v1.bits(), v2.bits()) {
        (8, 8) => Value::from(f8(v1.unwrap_i8(), v2.unwrap_i8())),
        (16, 16) => Value::from(f16(v1.unwrap_i16(), v2.unwrap_i16())),
        (32, 32) => Value::from(f32(v1.unwrap_i32(), v2.unwrap_i32())),
        (64, 64) => Value::from(f64(v1.unwrap_i64(), v2.unwrap_i64())),
        (128, 128) => Value::from(f128(v1.unwrap_i128(), v2.unwrap_i128())),
        _ => todo!()
    }
}


impl From<IntPredicate> for CIntPredicate {
    fn from(predicate: IntPredicate) -> Self {
        match predicate {
            IntPredicate::EQ => CIntPredicate::EQ,
            IntPredicate::NE => CIntPredicate::NE,
            IntPredicate::UGT => CIntPredicate::UGT,
            IntPredicate::UGE => CIntPredicate::UGE,
            IntPredicate::ULT => CIntPredicate::ULT,
            IntPredicate::ULE => CIntPredicate::ULE,
            IntPredicate::SGT => CIntPredicate::SGT,
            IntPredicate::SGE => CIntPredicate::SGE,
            IntPredicate::SLT => CIntPredicate::SLT,
            IntPredicate::SLE => CIntPredicate::SLE,
        }
    }
}


impl From<FPPredicate> for CFPPredicate {
    fn from(predicate: FPPredicate) -> Self {
        use CFPPredicate::*;
        match predicate {
            FPPredicate::False => False,
            FPPredicate::OEQ => OEQ,
            FPPredicate::OGT => OGT,
            FPPredicate::OGE => OGE,
            FPPredicate::OLT => OLT,
            FPPredicate::OLE => OLE,
            FPPredicate::ONE => ONE,
            FPPredicate::ORD => ORD,
            FPPredicate::UNO => UNO,
            FPPredicate::UEQ => UEQ,
            FPPredicate::UGT => UGT,
            FPPredicate::UGE => UGE,
            FPPredicate::ULT => ULT,
            FPPredicate::ULE => ULE,
            FPPredicate::UNE => UNE,
            FPPredicate::True => True,
        }
    }
}
