use crate::memory::Memory;
use crate::value::Value;
use crate::ctx::{Ctx, ThreadCtx};
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::layout::Layout;
use std::iter::repeat;
use crate::exec::Process;
use std::hint::unreachable_unchecked;
use std::cmp::Ordering;

pub struct NativeFunction {
    pub name: String,
    pub imp: Box<dyn Fn(&mut Process, ThreadCtx, &[&Value]) -> Value>,
}

impl Debug for NativeFunction {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFunction")
            .field("name", &self.name)
            .finish()
    }
}

macro_rules! overflow_binop {
    ($uop:expr, $sop:expr, $wrapping:ident, $checked:ident) => {
        vec![
            overflow_binop!($uop, $wrapping, $checked, "i8", unwrap_u8),
            overflow_binop!($uop, $wrapping, $checked, "i16", unwrap_u16),
            overflow_binop!($uop, $wrapping, $checked, "i32", unwrap_u32),
            overflow_binop!($uop, $wrapping, $checked, "i64", unwrap_u64),
            overflow_binop!($uop, $wrapping, $checked, "i128", unwrap_u128),
            overflow_binop!($sop, $wrapping, $checked, "i8", unwrap_i8),
            overflow_binop!($sop, $wrapping, $checked, "i16", unwrap_i16),
            overflow_binop!($sop, $wrapping, $checked, "i32", unwrap_i32),
            overflow_binop!($sop, $wrapping, $checked, "i64", unwrap_i64),
            overflow_binop!($sop, $wrapping, $checked, "i128", unwrap_i128)
        ]
    };
    ($op:expr, $wrapping:ident, $checked:ident, $ty:expr, $unwrap:ident) => {
        NativeFunction {
            name: format!("llvm.{}.with.overflow.{}", $op, $ty),
            imp: Box::new(|_, _, args| {
                let (x, y) = (args[0].$unwrap(), args[1].$unwrap());
                Value::aggregate([
                                     Value::from(x.$wrapping(y)),
                                     Value::from(x.$checked(y).is_none())
                                 ].iter().cloned(), false)
            }),
        }
    }
}

impl NativeFunction {
    pub(crate) fn builtins() -> Vec<NativeFunction> {
        let mut result = vec![];
        result.append(&mut vec![
            NativeFunction {
                name: "llvm.dbg.declare".to_string(),
                imp: Box::new(|_, _, _| {
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "signal".to_string(),
                imp: Box::new(|p, tctx, _| {
                    Value::new(p.ctx().ptr_bits, 0)
                }),
            },
            NativeFunction {
                name: "sysconf".to_string(),
                imp: Box::new(|p, tctx, args| {
                    Value::from(match args[0].unwrap_u32() {
                        29 => unsafe { libc::sysconf(libc::_SC_PAGESIZE) },
                        x => todo!("{:?}", x),
                    } as u64)
                }),
            },
            NativeFunction {
                name: "pthread_rwlock_rdlock".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_rwlock_unlock".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_lock".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_unlock".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_unlock".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_init".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_settype".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_init".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_destroy".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_destroy".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "_tlv_atexit".to_string(),
                imp: Box::new(|p, tctx, _| {
                    //TODO synchronize lock
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "llvm.expect.i1".to_string(),
                imp: Box::new(|p, tctx, xs| {
                    xs[0].clone()
                }),
            },
            NativeFunction {
                name: "llvm.cttz.i64".to_string(),
                imp: Box::new(|p, tctx, args| {
                    Value::from(args[0].as_u64().trailing_zeros() as u64)
                }),
            },
            NativeFunction {
                name: "llvm.memcpy.p0i8.p0i8.i64".to_string(),
                imp: Box::new(|p, tctx, args| {
                    p.memcpy(tctx, args[0], args[1], args[2].as_u64());
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "llvm.ctpop.i64".to_string(),
                imp: Box::new(|p, tctx, args| {
                    Value::from(args[0].unwrap_u64().count_ones() as u64)
                }),
            },
            NativeFunction {
                name: "llvm.assume".to_string(),
                imp: Box::new(|p, tctx, args| {
                    assert!(args[0].unwrap_bool());
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "write".to_string(),
                imp: Box::new(|p, tctx, args| {
                    let (fd, buf, len) = (args[0], args[1], args[2].as_u64());
                    let value = if len > 0 {
                        p.load(tctx, &buf, Layout::from_bytes(len, 1), None)
                    } else {
                        Value::from(())
                    };
                    let value = value.bytes();
                    let string = String::from_utf8(value.to_vec()).unwrap();
                    print!("{}", string);
                    p.ctx().value_from_address(len)
                }),
            },
            NativeFunction {
                name: "llvm.trap".to_string(),
                imp: Box::new(|p, tctx, args| {
                    panic!("It's a trap!");
                }),
            },
            NativeFunction {
                name: "pthread_self".to_string(),
                imp: Box::new(|p, tctx, args| {
                    p.ctx().value_from_address(0)
                }),
            },
            NativeFunction {
                name: "pthread_get_stackaddr_np".to_string(),
                imp: Box::new(|p, tctx, args| {
                    p.ctx().value_from_address(0)
                }),
            },
            NativeFunction {
                name: "pthread_get_stacksize_np".to_string(),
                imp: Box::new(|p, tctx, args| {
                    p.ctx().value_from_address(0)
                }),
            },
            NativeFunction {
                name: "mmap".to_string(),
                imp: Box::new(|p, tctx, args| {
                    let (addr, length, prot, flags, fd, offset) =
                        (args[0].as_u64(),
                         args[1].as_u64(),
                         args[2].unwrap_u32(),
                         args[3].unwrap_u32(),
                         args[4].unwrap_u32(),
                         args[5].as_u64());
                    //memory.alloc(Layout::from_size_align(length,ctx.page_size))
                    p.ctx().value_from_address(addr)
                }),
            },
            NativeFunction {
                name: "mprotect".to_string(),
                imp: Box::new(|p, tctx, args| {
                    let (addr, length, prot) =
                        (args[0].as_u64(),
                         args[1].as_u64(),
                         args[2].unwrap_u32());
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "llvm.memset.p0i8.i64".to_string(),
                imp: Box::new(|p, tctx, args| {
                    let (addr, val, len) = (args[0], args[1], args[2].unwrap_u64());
                    p.store(tctx, addr,
                            &Value::from_bytes(&vec![val.unwrap_u8(); len as usize],
                                               Layout::from_bytes(len, 1)),
                            None);
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "sigaction".to_string(),
                imp: Box::new(|p, tctx, args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "sigaltstack".to_string(),
                imp: Box::new(|p, tctx, args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "__rust_alloc".to_string(),
                imp: Box::new(|p, tctx, args| {
                    p.alloc(tctx, Layout::from_bytes(args[0].as_u64(), args[1].as_u64()))
                }),
            },
            NativeFunction {
                name: "__rust_realloc".to_string(),
                imp: Box::new(|p, tctx, args| {
                    p.realloc(tctx, args[0],
                              Layout::from_bytes(args[1].as_u64(), args[2].as_u64()),
                              Layout::from_bytes(args[3].as_u64(), args[2].as_u64()))
                }),
            },
            NativeFunction {
                name: "__rust_dealloc".to_string(),
                imp: Box::new(|p, tctx, args| {
                    p.free(tctx, args[0]);
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "memchr".to_string(),
                imp: Box::new(|p, tctx, args| {
                    for i in 0..args[2].as_u64() {
                        let ptr = p.ctx().value_from_address(args[0].as_u64() + i);
                        let v = p.load(tctx, &ptr, Layout::from_bytes(1, 1), None);
                        if v.unwrap_u8() as u32 == args[1].unwrap_u32() {
                            return ptr;
                        }
                    }
                    p.ctx().value_from_address(0)
                }),
            },
            NativeFunction {
                name: "strlen".to_string(),
                imp: Box::new(|p, tctx, args| {
                    for i in 0.. {
                        let ptr = p.ctx().value_from_address(args[0].as_u64() + i);
                        let v = p.load(tctx, &ptr, Layout::from_bytes(1, 1), None);
                        if v.unwrap_u8() as u32 == 0 {
                            return p.ctx().value_from_address(i);
                        }
                    }
                    unreachable!();
                }),
            },
            NativeFunction {
                name: "memcmp".to_string(),
                imp: Box::new(|p, tctx, args| {
                    for i in 0..args[2].as_u64() {
                        let ptr1 = p.ctx().value_from_address(args[0].as_u64() + i);
                        let ptr2 = p.ctx().value_from_address(args[1].as_u64() + i);
                        let v1 = p.load(tctx, &ptr1, Layout::from_bytes(1, 1), None);
                        let v2 = p.load(tctx, &ptr2, Layout::from_bytes(1, 1), None);
                        match v1.unwrap_u8().cmp(&v2.unwrap_u8()) {
                            Ordering::Less => return Value::from(-1i32),
                            Ordering::Equal => {}
                            Ordering::Greater => return Value::from(1i32),
                        }
                    }
                    Value::from(0i32)
                }),
            },
            NativeFunction {
                name: "getenv".to_string(),
                imp: Box::new(|p, tctx, args| {
                    let mut cstr = vec![];
                    for i in args[0].as_u64().. {
                        let c = p.load(tctx, &Value::from(i), Layout::of_int(8), None).unwrap_u8();
                        if c == 0 { break; } else { cstr.push(c) }
                    }
                    let str = String::from_utf8(cstr).unwrap();
                    match str.as_str() {
                        "RUST_BACKTRACE" => {
                            let mut cstr = b"full\0";
                            let layout = Layout::from_bytes(cstr.len() as u64, 1);
                            let res = p.alloc(tctx, layout);
                            p.store(tctx, &res, &Value::from_bytes(cstr, layout), None);
                            res
                        }
                        _ => p.ctx().value_from_address(0)
                    }
                }),
            },
            NativeFunction {
                name: "getcwd".to_string(),
                imp: Box::new(|p, tctx, args| {
                    let buf = args[0].as_u64();
                    let size = args[1].as_u64();
                    if buf != 0 {
                        if size != 0 {
                            p.store(tctx, &args[0], &Value::from(0u8), None);
                        }
                        args[0].clone()
                    } else {
                        let layout = Layout::of_int(8);
                        let res = p.alloc(tctx, layout);
                        p.store(tctx, &res, &Value::from(0u8), None);
                        res
                    }
                }),
            },
            NativeFunction {
                name: "_Unwind_RaiseException".to_string(),
                imp: Box::new(|p, tctx, args| {
                    panic!("Unwinding!");
                }),
            },
            NativeFunction {
                name: "_Unwind_Backtrace".to_string(),
                imp: Box::new(|p, tctx, args| {
                    Value::from(0u32)
                }),
            },
        ]);
        result.append(&mut overflow_binop!("uadd", "sadd", wrapping_add, checked_add));
        result.append(&mut overflow_binop!("umul", "smul", wrapping_mul, checked_mul));
        result.append(&mut overflow_binop!("usub", "ssub", wrapping_sub, checked_sub));
        result
    }
}
