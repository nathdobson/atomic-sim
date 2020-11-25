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
use crate::data_flow::ComputeArgs;

pub struct NativeFunction {
    pub name: String,
    pub imp: Box<dyn Fn(ComputeArgs) -> Value>,
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
            imp: Box::new(|args| {
                let (x, y) = (args.args[0].$unwrap(), args.args[1].$unwrap());
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
                imp: Box::new(|args| {
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "signal".to_string(),
                imp: Box::new(|args| {
                    Value::new(args.ctx().ptr_bits, 0)
                }),
            },
            NativeFunction {
                name: "sysconf".to_string(),
                imp: Box::new(|args| {
                    Value::from(match args.args[0].unwrap_u32() {
                        29 => unsafe { libc::sysconf(libc::_SC_PAGESIZE) },
                        x => todo!("{:?}", x),
                    } as u64)
                }),
            },
            NativeFunction {
                name: "pthread_rwlock_rdlock".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_rwlock_unlock".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_lock".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_unlock".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_unlock".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_init".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_settype".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_init".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_destroy".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_destroy".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_self".to_string(),
                imp: Box::new(|args| {
                    args.ctx().value_from_address(0)
                }),
            },
            NativeFunction {
                name: "pthread_get_stackaddr_np".to_string(),
                imp: Box::new(|args| {
                    args.ctx().value_from_address(0)
                }),
            },
            NativeFunction {
                name: "pthread_get_stacksize_np".to_string(),
                imp: Box::new(|args| {
                    args.ctx().value_from_address(0)
                }),
            },
            NativeFunction {
                name: "pthread_attr_init".to_string(),
                imp: Box::new(|args| {
                    let value = [0xFFu8; 64];
                    args.process.store(args.tctx, args.args[0], &Value::from_bytes(&value, Layout::from_bytes(value.len() as u64, 1)), None);
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "pthread_attr_setstacksize".to_string(),
                imp: Box::new(|args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "pthread_create".to_string(),
                imp: Box::new(|args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "pthread_attr_destroy".to_string(),
                imp: Box::new(|args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "pthread_join".to_string(),
                imp: Box::new(|args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "_tlv_atexit".to_string(),
                imp: Box::new(|args| {
                    //TODO synchronize lock
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "llvm.expect.i1".to_string(),
                imp: Box::new(|args| {
                    args.args[0].clone()
                }),
            },
            NativeFunction {
                name: "llvm.cttz.i64".to_string(),
                imp: Box::new(|args| {
                    Value::from(args.args[0].as_u64().trailing_zeros() as u64)
                }),
            },
            NativeFunction {
                name: "llvm.memcpy.p0i8.p0i8.i64".to_string(),
                imp: Box::new(|args| {
                    args.process.memcpy(args.tctx, args.args[0], args.args[1], args.args[2].as_u64());
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "llvm.ctpop.i64".to_string(),
                imp: Box::new(|args| {
                    Value::from(args.args[0].unwrap_u64().count_ones() as u64)
                }),
            },
            NativeFunction {
                name: "llvm.assume".to_string(),
                imp: Box::new(|args| {
                    assert!(args.args[0].unwrap_bool());
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "write".to_string(),
                imp: Box::new(|args| {
                    let (fd, buf, len) = (args.args[0], args.args[1], args.args[2].as_u64());
                    let value = if len > 0 {
                        args.process.load(args.tctx, &buf, Layout::from_bytes(len, 1), None)
                    } else {
                        Value::from(())
                    };
                    let value = value.bytes();
                    let string = String::from_utf8(value.to_vec()).unwrap();
                    print!("{}", string);
                    args.ctx().value_from_address(len)
                }),
            },
            NativeFunction {
                name: "llvm.trap".to_string(),
                imp: Box::new(|args| {
                    panic!("It's a trap!");
                }),
            },
            NativeFunction {
                name: "mmap".to_string(),
                imp: Box::new(|args| {
                    let (addr, length, prot, flags, fd, offset) =
                        (args.args[0].as_u64(),
                         args.args[1].as_u64(),
                         args.args[2].unwrap_u32(),
                         args.args[3].unwrap_u32(),
                         args.args[4].unwrap_u32(),
                         args.args[5].as_u64());
                    //memory.alloc(Layout::from_size_align(length,ctx.page_size))
                    args.ctx().value_from_address(addr)
                }),
            },
            NativeFunction {
                name: "mprotect".to_string(),
                imp: Box::new(|args| {
                    let (addr, length, prot) =
                        (args.args[0].as_u64(),
                         args.args[1].as_u64(),
                         args.args[2].unwrap_u32());
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "llvm.memset.p0i8.i64".to_string(),
                imp: Box::new(|args| {
                    let (addr, val, len) = (args.args[0], args.args[1], args.args[2].unwrap_u64());
                    args.process.store(args.tctx, addr,
                            &Value::from_bytes(&vec![val.unwrap_u8(); len as usize],
                                               Layout::from_bytes(len, 1)),
                            None);
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "sigaction".to_string(),
                imp: Box::new(|args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "sigaltstack".to_string(),
                imp: Box::new(|args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "__rust_alloc".to_string(),
                imp: Box::new(|args| {
                    args.process.alloc(args.tctx, Layout::from_bytes(args.args[0].as_u64(), args.args[1].as_u64()))
                }),
            },
            NativeFunction {
                name: "__rust_realloc".to_string(),
                imp: Box::new(|args| {
                    args.process.realloc(args.tctx, args.args[0],
                              Layout::from_bytes(args.args[1].as_u64(), args.args[2].as_u64()),
                              Layout::from_bytes(args.args[3].as_u64(), args.args[2].as_u64()))
                }),
            },
            NativeFunction {
                name: "__rust_dealloc".to_string(),
                imp: Box::new(|args| {
                    args.process.free(args.tctx, args.args[0]);
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "memchr".to_string(),
                imp: Box::new(|args| {
                    for i in 0..args.args[2].as_u64() {
                        let ptr = args.ctx().value_from_address(args.args[0].as_u64() + i);
                        let v = args.process.load(args.tctx, &ptr, Layout::from_bytes(1, 1), None);
                        if v.unwrap_u8() as u32 == args.args[1].unwrap_u32() {
                            return ptr;
                        }
                    }
                    args.ctx().value_from_address(0)
                }),
            },
            NativeFunction {
                name: "strlen".to_string(),
                imp: Box::new(|args| {
                    for i in 0.. {
                        let ptr = args.ctx().value_from_address(args.args[0].as_u64() + i);
                        let v = args.process.load(args.tctx, &ptr, Layout::from_bytes(1, 1), None);
                        if v.unwrap_u8() as u32 == 0 {
                            return args.ctx().value_from_address(i);
                        }
                    }
                    unreachable!();
                }),
            },
            NativeFunction {
                name: "memcmp".to_string(),
                imp: Box::new(|args| {
                    for i in 0..args.args[2].as_u64() {
                        let ptr1 = args.ctx().value_from_address(args.args[0].as_u64() + i);
                        let ptr2 = args.ctx().value_from_address(args.args[1].as_u64() + i);
                        let v1 = args.process.load(args.tctx, &ptr1, Layout::from_bytes(1, 1), None);
                        let v2 = args.process.load(args.tctx, &ptr2, Layout::from_bytes(1, 1), None);
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
                imp: Box::new(|args| {
                    let mut cstr = vec![];
                    for i in args.args[0].as_u64().. {
                        let c = args.process.load(args.tctx, &Value::from(i), Layout::of_int(8), None).unwrap_u8();
                        if c == 0 { break; } else { cstr.push(c) }
                    }
                    let str = String::from_utf8(cstr).unwrap();
                    match str.as_str() {
                        "RUST_BACKTRACE" => {
                            let mut cstr = b"full\0";
                            let layout = Layout::from_bytes(cstr.len() as u64, 1);
                            let res = args.process.alloc(args.tctx, layout);
                            args.process.store(args.tctx, &res, &Value::from_bytes(cstr, layout), None);
                            res
                        }
                        _ => args.ctx().value_from_address(0)
                    }
                }),
            },
            NativeFunction {
                name: "getcwd".to_string(),
                imp: Box::new(|args| {
                    let buf = args.args[0].as_u64();
                    let size = args.args[1].as_u64();
                    if buf != 0 {
                        if size != 0 {
                            args.process.store(args.tctx, &args.args[0], &Value::from(0u8), None);
                        }
                        args.args[0].clone()
                    } else {
                        let layout = Layout::of_int(8);
                        let res = args.process.alloc(args.tctx, layout);
                        args.process.store(args.tctx, &res, &Value::from(0u8), None);
                        res
                    }
                }),
            },
            NativeFunction {
                name: "_Unwind_RaiseException".to_string(),
                imp: Box::new(|args| {
                    panic!("Unwinding!");
                }),
            },
            NativeFunction {
                name: "_Unwind_Backtrace".to_string(),
                imp: Box::new(|args| {
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
