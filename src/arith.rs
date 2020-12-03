use llvm_ir::{TypeRef, Type, IntPredicate};
use crate::value::Value;
use crate::ctx::Ctx;
use std::ops::Add;
use std::iter;
use crate::layout::{Layout, Packing};

#[derive(Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    UDiv,
    SDiv,
    URem,
    SRem,
    Xor,
    And,
    Or,
    Shl,
    LShr,
    AShr,
    ICmp(IntPredicate),
    Xchg,
}

#[derive(Debug)]
pub enum UnOp {
    Scast,
    Ucast,
}

macro_rules! repeat5 {
    ($x:expr)=>(($x,$x,$x,$x,$x))
}

impl BinOp {
    pub fn call<'ctx>(&self, ctx: &Ctx<'ctx>, ty1: &TypeRef, v1: &Value, ty2: &TypeRef, v2: &Value) -> Value {
        match (&**ty1, &**ty2) {
            (Type::IntegerType { .. }, Type::IntegerType { .. })
            | (Type::PointerType { .. }, Type::PointerType { .. }) => {
                self.call_scalar(v1, v2)
            }

            (Type::VectorType { element_type: e1, num_elements: n1 },
                Type::VectorType { element_type: e2, num_elements: n2 }) => {
                assert_eq!(n1, n2);
                Value::aggregate((0..*n1).map(|i| {
                    self.call(ctx,
                              e1, &ctx.extract_value(ty1, v1, iter::once(i as i64)),
                              e2, &ctx.extract_value(ty2, v2, iter::once(i as i64)))
                }), Packing::Bit)
            }
            _ => todo!("{:?} {:?} {:?}", ty1, self, ty2)
        }
    }
    fn call_scalar(&self, v1: &Value, v2: &Value) -> Value {
        match self {
            BinOp::Add => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_add(y))),
            BinOp::Sub => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_sub(y))),
            BinOp::Mul => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_mul(y))),
            BinOp::UDiv => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_div(y))),
            BinOp::SDiv => Self::call_signed(v1, v2, repeat5!(|x,y| x.wrapping_div(y))),
            BinOp::URem => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x.wrapping_rem(y))),
            BinOp::SRem => Self::call_signed(v1, v2, repeat5!(|x,y| x.wrapping_rem(y))),
            BinOp::Xor => Self::call_unsigned(v1, v2, |x, y| x ^ y, repeat5!(|x,y| x^y)),
            BinOp::And => Self::call_unsigned(v1, v2, |x, y| x & y, repeat5!(|x,y| x&y)),
            BinOp::Or => Self::call_unsigned(v1, v2, |x, y| x | y, repeat5!(|x,y| x|y)),
            BinOp::Shl => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x<<y)),
            BinOp::LShr => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x>>y)),
            BinOp::AShr => Self::call_signed(v1, v2, repeat5!(|x,y| x>>y)),
            BinOp::Xchg => v2.clone(),
            BinOp::ICmp(predicate) => {
                match predicate {
                    IntPredicate::EQ => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x==y)),
                    IntPredicate::NE => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x!=y)),
                    IntPredicate::UGT => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x>y)),
                    IntPredicate::UGE => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x>=y)),
                    IntPredicate::ULT => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x<y)),
                    IntPredicate::ULE => Self::call_unsigned(v1, v2, |_, _| todo!(), repeat5!(|x,y| x<=y)),
                    IntPredicate::SGT => Self::call_signed(v1, v2, repeat5!(|x,y| x>y)),
                    IntPredicate::SGE => Self::call_signed(v1, v2, repeat5!(|x,y| x>=y)),
                    IntPredicate::SLT => Self::call_signed(v1, v2, repeat5!(|x,y| x<y)),
                    IntPredicate::SLE => Self::call_signed(v1, v2, repeat5!(|x,y| x<=y)),
                }
            }
        }
    }
    fn call_unsigned<T1, T8, T16, T32, T64, T128>
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

    fn call_signed<T8, T16, T32, T64, T128>
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
}

impl UnOp {
    pub fn call<'ctx>(&self, ctx: &Ctx<'ctx>, ty1: &TypeRef, v1: &Value, ty2: &TypeRef) -> Value {
        match self {
            UnOp::Scast => Self::scast(ctx, ty1, v1, ty2),
            UnOp::Ucast => Self::ucast(ctx, ty1, v1, ty2),
        }
    }
    pub fn ucast<'ctx>(ctx: &Ctx<'ctx>, ty1: &TypeRef, v1: &Value, ty2: &TypeRef) -> Value {
        v1.ucast(ctx.layout(ty2))
    }
    pub fn scast<'ctx>(ctx: &Ctx<'ctx>, ty1: &TypeRef, v1: &Value, ty2: &TypeRef) -> Value {
        match (&**ty1, &**ty2) {
            (Type::IntegerType { .. } | Type::PointerType { .. },
                Type::IntegerType { .. } | Type::PointerType { .. })
            => {
                Self::scast_scalar(v1, ctx.layout(ty2))
            }

            (Type::VectorType { element_type: e1, num_elements: n1 },
                Type::VectorType { element_type: e2, num_elements: n2 }) => {
                assert_eq!(n1, n2);
                Value::aggregate((0..*n1).map(|i| {
                    Self::scast(ctx,
                                e1, &ctx.extract_value(ty1, v1, iter::once(i as i64)),
                                e2)
                }), Packing::Bit)
            }
            _ => todo!("{:?} {:?}", ty1, ty2),
        }
    }
    pub fn scast_scalar(v1: &Value, layout: Layout) -> Value {
        match (v1.bits(), layout.bits()) {
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