use std::rc::Rc;

use crate::data::ComputeCtx;
use crate::flow::FlowCtx;
use crate::function::Func;
use crate::native::{Addr, native_bomb, native_comp_new};
use crate::native_comp;
use crate::native_fn;
use crate::util::future::FutureExt;
use crate::value::Value;

macro_rules! overflow_binop {
    ($uop:expr, $sop:expr, $wrapping:ident, $checked:ident) => {
        vec![
            overflow_binop!($uop, $wrapping, $checked, "i8", u8),
            overflow_binop!($uop, $wrapping, $checked, "i16", u16),
            overflow_binop!($uop, $wrapping, $checked, "i32", u32),
            overflow_binop!($uop, $wrapping, $checked, "i64", u64),
            overflow_binop!($uop, $wrapping, $checked, "i128", u128),
            overflow_binop!($sop, $wrapping, $checked, "i8", i8),
            overflow_binop!($sop, $wrapping, $checked, "i16", i16),
            overflow_binop!($sop, $wrapping, $checked, "i32", i32),
            overflow_binop!($sop, $wrapping, $checked, "i64", i64),
            overflow_binop!($sop, $wrapping, $checked, "i128", i128)
        ]
    };
    ($op:expr, $wrapping:ident, $checked:ident, $llvm_ty:expr, $rust_ty:ident) => {
        {
            fn imp(comp: &ComputeCtx, (x, y):($rust_ty, $rust_ty))->Value{
                let types = comp.process.types.clone();
                let v1 = Value::from(x.$wrapping(y));
                let v2 = Value::from(x.$checked(y).is_none());
                let class = types.struc(vec![types.int(v1.bits()), types.int(v2.bits())], false);
                Value::aggregate(&class, [v1, v2].iter().cloned())
            }
            native_comp!(&**Box::leak(Box::new(format!("llvm.{}.with.overflow.{}", $op, $llvm_ty))), imp)
        }
    }
}

macro_rules! sat_binop {
    ($uop:expr, $sop:expr, $method:ident) => {
        vec![
            sat_binop!($uop, $method, "i8", u8),
            sat_binop!($uop, $method, "i16", u16),
            sat_binop!($uop, $method, "i32", u32),
            sat_binop!($uop, $method, "i64", u64),
            sat_binop!($uop, $method, "i128", u128),
            sat_binop!($sop, $method, "i8", i8),
            sat_binop!($sop, $method, "i16", i16),
            sat_binop!($sop, $method, "i32", i32),
            sat_binop!($sop, $method, "i64", i64),
            sat_binop!($sop, $method, "i128", i128)
        ]
    };
    ($op:expr, $method:ident, $llvm_ty:expr, $rust_ty:ident) => {
        {
            fn imp(comp: &ComputeCtx, (x, y): ($rust_ty, $rust_ty)) -> $rust_ty{
                x.$method(y)
            }
            native_comp_new(&**Box::leak(Box::new(format!("llvm.{}.{}", $op, $llvm_ty))), imp)
        }
    }
}

macro_rules! unop {
    ($op:expr, $method:ident) => {
        vec![
            unop!($op, $method, "i8", u8),
            unop!($op, $method, "i16", u16),
            unop!($op, $method, "i32", u32),
            unop!($op, $method, "i64", u64),
            unop!($op, $method, "i128", u128),
        ]
    };
    ($op:expr, $method:ident, $llvm_ty:expr, $rust_ty:ty) => {
        {
            fn imp(comp:&ComputeCtx, (x,): ($rust_ty,)) -> $rust_ty{
                x.$method() as $rust_ty
            }
            native_comp_new(&**Box::leak(Box::new(format!("llvm.{}.{}", $op, $llvm_ty))), imp)
        }
    }
}

fn llvm_dbg_declare(comp: &ComputeCtx, _: (Value, Value, Value)) {}

fn llvm_dbg_value(comp: &ComputeCtx, _: (Value, Value, Value)) {}

fn llvm_expect_i1(comp: &ComputeCtx, (arg, _): (Value, Value)) -> Value { arg }

fn llvm_lifetime_start_p0i8(comp: &ComputeCtx, _: (Value, Value)) {}

fn llvm_lifetime_end_p0i8(comp: &ComputeCtx, _: (Value, Value)) {}

async fn llvm_memcpy_p0i8_p0i8_i64(flow: &FlowCtx, (Addr(dst), Addr(src), Addr(cnt), volatile): (Addr, Addr, Addr, bool)) {
    flow.memcpy(dst, src, cnt).await;
}

async fn llvm_memmove_p0i8_p0i8_i64(flow: &FlowCtx, (Addr(dst), Addr(src), Addr(cnt)): (Addr, Addr, Addr)) {
    flow.memcpy(dst, src, cnt).await;
}

fn llvm_ctpop_i64(comp: &ComputeCtx, (x, ): (u64, )) -> u64 {
    x.count_ones() as u64
}

fn llvm_assume(comp: &ComputeCtx, (arg, ): (bool, )) {
    assert!(arg);
}

fn llvm_trap(comp: &ComputeCtx, (): ()) {
    panic!("It's a trap!");
}

async fn llvm_memset_p0i8_i64(flow: &FlowCtx, (Addr(addr), val, Addr(len), volatile): (Addr, u8, Addr, bool)) {
    flow.store(addr,
               &Value::from_bytes_exact(&vec![val; len as usize]),
    ).await;
}

fn llvm_fshl_i64(comp: &ComputeCtx, (a, b, s): (u64, u64, u64)) -> u64 {
    ((((a as u128) << 64 | (b as u128)) << (s as u128)) >> 64) as u64
}

pub fn builtins() -> Vec<Rc<dyn Func>> {
    let mut result = vec![
        native_comp!("llvm.dbg.declare", llvm_dbg_declare),
        native_comp!("llvm.dbg.value", llvm_dbg_value),
        native_comp!("llvm.expect.i1", llvm_expect_i1),
        native_comp!("llvm.lifetime.start.p0i8", llvm_lifetime_start_p0i8),
        native_comp!("llvm.lifetime.end.p0i8", llvm_lifetime_end_p0i8),
        native_fn!("llvm.memcpy.p0i8.p0i8.i64", llvm_memcpy_p0i8_p0i8_i64),
        native_fn!("llvm.memmove.p0i8.p0i8.i64", llvm_memmove_p0i8_p0i8_i64),
        native_comp!("llvm.ctpop.i64", llvm_ctpop_i64),
        native_comp!("llvm.assume", llvm_assume),
        native_comp!("llvm.trap", llvm_trap),
        native_fn!("llvm.memset.p0i8.i64", llvm_memset_p0i8_i64),
        native_comp!("llvm.fshl.i64", llvm_fshl_i64),
    ];
    result.append(&mut overflow_binop!("uadd", "sadd", wrapping_add, checked_add));
    result.append(&mut overflow_binop!("umul", "smul", wrapping_mul, checked_mul));
    result.append(&mut overflow_binop!("usub", "ssub", wrapping_sub, checked_sub));
    result.append(&mut sat_binop!("usub.sat", "ssub.sat", saturating_sub));
    result.append(&mut unop!("cttz", trailing_zeros));
    result.append(&mut unop!("ctlz", leading_zeros));
    for bomb in [
        "llvm.bswap.i128",
        "llvm.bswap.i16",
        "llvm.bswap.i32",
        "llvm.bswap.v8i16",
        "llvm.ctpop.i32",
        "llvm.fshr.i32",
        "llvm.uadd.sat.i64",
    ].iter() {
        result.push(native_bomb(bomb));
    }
    result
}