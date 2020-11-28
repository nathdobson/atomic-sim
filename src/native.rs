use std::cmp::Ordering;
use crate::layout::Layout;
use crate::value::Value;
use crate::function::Func;
use crate::data::{ComputeArgs, DataFlow, Thunk};
use std::rc::Rc;
use futures::future::LocalBoxFuture;
use std::marker::PhantomData;

struct NativeComp<'ctx> {
    name: &'ctx str,
    phantom: PhantomData<&'ctx ()>,
    imp: Rc<dyn 'ctx + for<'comp> Fn(ComputeArgs<'ctx, 'comp>) -> Value>,
}

impl<'ctx> Func<'ctx> for NativeComp<'ctx> {
    fn name(&self) -> &'ctx str {
        self.name
    }

    fn call_imp<'a>(&'a self, data: &'a DataFlow<'ctx>, args: Vec<Rc<Thunk<'ctx>>>) -> LocalBoxFuture<'a, Option<Rc<Thunk<'ctx>>>> {
        let imp = self.imp.clone();
        Box::pin(async move {
            Some(data.add_thunk(args, move |args| imp(args)).await)
        })
    }

    // fn call_imp<'a>(&'ctx self, data: &'a DataFlow<'ctx>, args: Vec<Rc<Thunk<'ctx>>>) -> LocalBoxFuture<'a, Rc<Thunk<'ctx>>> {
    //     Box::pin(data.add_thunk(args, &self.imp))
    // }
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
        native_comp_new(
            &**Box::leak(Box::new(format!("llvm.{}.with.overflow.{}", $op, $ty))),
            move |args| {
                let (x, y) = (args.args[0].$unwrap(), args.args[1].$unwrap());
                Value::aggregate([
                                     Value::from(x.$wrapping(y)),
                                     Value::from(x.$checked(y).is_none())
                                 ].iter().cloned(), false)
            }
        )
    }
}

pub fn native_comp_new<'ctx>(name: &'ctx str, imp: impl 'static + for<'comp> Fn(ComputeArgs<'ctx, 'comp>) -> Value) -> Rc<dyn 'ctx + Func<'ctx>> {
    Rc::new(NativeComp {
        name: name,
        imp: Rc::new(imp),
        phantom: Default::default(),
    })
}

pub fn builtins<'ctx>() -> Vec<Rc<dyn 'ctx + Func<'ctx>>> {
    let mut result: Vec<Rc<dyn 'ctx + Func<'ctx>>> = vec![];
    result.append(&mut vec![
        native_comp_new("llvm.dbg.declare", |args: ComputeArgs| {
            Value::from(())
        }),
        native_comp_new("signal", |args| {
            Value::new(args.ctx().ptr_bits, 0)
        }),
        native_comp_new("sysconf", |args| {
            Value::from(match args.args[0].unwrap_u32() {
                29 => unsafe { libc::sysconf(libc::_SC_PAGESIZE) },
                x => todo!("{:?}", x),
            } as u64)
        }),
        native_comp_new("pthread_rwlock_rdlock", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_rwlock_unlock", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_lock", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_unlock", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_unlock", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutexattr_init", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutexattr_settype", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_init", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutexattr_destroy", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_destroy", |args| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_self", |args| {
            args.ctx().value_from_address(0)
        }),
        native_comp_new("pthread_get_stackaddr_np", |args| {
            args.ctx().value_from_address(0)
        }),
        native_comp_new("pthread_get_stacksize_np", |args| {
            args.ctx().value_from_address(0)
        }),
        native_comp_new("pthread_attr_init", |args| {
            let value = [0xFFu8; 64];
            args.process.store(args.tctx, args.args[0], &Value::from_bytes(&value, Layout::from_bytes(value.len() as u64, 1)), None);
            Value::from(0u32)
        }),
        native_comp_new("pthread_attr_setstacksize", |args| {
            Value::from(0u32)
        }),
        native_comp_new("pthread_create", |args| {
            Value::from(0u32)
        }),
        native_comp_new("pthread_attr_destroy", |args| {
            Value::from(0u32)
        }),
        native_comp_new("pthread_join", |args| {
            Value::from(0u32)
        }),
        native_comp_new("_tlv_atexit", |args| {
            //TODO synchronize lock
            Value::from(())
        }),
        native_comp_new("llvm.expect.i1", |args| {
            args.args[0].clone()
        }),
        native_comp_new("llvm.cttz.i64", |args| {
            Value::from(args.args[0].as_u64().trailing_zeros() as u64)
        }),
        native_comp_new("llvm.memcpy.p0i8.p0i8.i64", |args| {
            args.process.memcpy(args.tctx, args.args[0], args.args[1], args.args[2].as_u64());
            Value::from(())
        }),
        native_comp_new("llvm.ctpop.i64", |args| {
            Value::from(args.args[0].unwrap_u64().count_ones() as u64)
        }),
        native_comp_new("llvm.assume", |args| {
            assert!(args.args[0].unwrap_bool());
            Value::from(())
        }),
        native_comp_new("write", |args| {
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
        native_comp_new("llvm.trap", |args| {
            panic!("It's a trap!");
        }),
        native_comp_new("mmap", |args| {
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
        native_comp_new("mprotect", |args| {
            let (addr, length, prot) =
                (args.args[0].as_u64(),
                 args.args[1].as_u64(),
                 args.args[2].unwrap_u32());
            Value::from(0u32)
        }),
        native_comp_new("llvm.memset.p0i8.i64", |args| {
            let (addr, val, len) = (args.args[0], args.args[1], args.args[2].unwrap_u64());
            args.process.store(args.tctx, addr,
                               &Value::from_bytes(&vec![val.unwrap_u8(); len as usize],
                                                  Layout::from_bytes(len, 1)),
                               None);
            Value::from(())
        }),
        native_comp_new("sigaction", |args| {
            Value::from(0u32)
        }),
        native_comp_new("sigaltstack", |args| {
            Value::from(0u32)
        }),
        native_comp_new("__rust_alloc", |args| {
            args.process.alloc(args.tctx, Layout::from_bytes(args.args[0].as_u64(), args.args[1].as_u64()))
        }),
        native_comp_new("__rust_realloc", |args| {
            args.process.realloc(args.tctx, args.args[0],
                                 Layout::from_bytes(args.args[1].as_u64(), args.args[2].as_u64()),
                                 Layout::from_bytes(args.args[3].as_u64(), args.args[2].as_u64()))
        }),
        native_comp_new("__rust_dealloc", |args| {
            args.process.free(args.tctx, args.args[0]);
            Value::from(())
        }),
        native_comp_new("memchr", |args| {
            for i in 0..args.args[2].as_u64() {
                let ptr = args.ctx().value_from_address(args.args[0].as_u64() + i);
                let v = args.process.load(args.tctx, &ptr, Layout::from_bytes(1, 1), None);
                if v.unwrap_u8() as u32 == args.args[1].unwrap_u32() {
                    return ptr;
                }
            }
            args.ctx().value_from_address(0)
        }),
        native_comp_new("strlen", |args| {
            for i in 0.. {
                let ptr = args.ctx().value_from_address(args.args[0].as_u64() + i);
                let v = args.process.load(args.tctx, &ptr, Layout::from_bytes(1, 1), None);
                if v.unwrap_u8() as u32 == 0 {
                    return args.ctx().value_from_address(i);
                }
            }
            unreachable!();
        }),
        native_comp_new("memcmp", |args| {
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
        native_comp_new("getenv", |args| {
            let mut cstr = vec![];
            for i in args.args[0].as_u64().. {
                let c = args.process.load(args.tctx, &Value::from(i), Layout::of_int(8), None).unwrap_u8();
                if c == 0 { break; } else { cstr.push(c) }
            }
            let str = String::from_utf8(cstr).unwrap();
            match str.as_str() {
                "RUST_BACKTRACE" => {
                    let cstr = b"full\0";
                    let layout = Layout::from_bytes(cstr.len() as u64, 1);
                    let res = args.process.alloc(args.tctx, layout);
                    args.process.store(args.tctx, &res, &Value::from_bytes(cstr, layout), None);
                    res
                }
                _ => args.ctx().value_from_address(0)
            }
        }),
        native_comp_new("getcwd", |args| {
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
        native_comp_new("_Unwind_RaiseException", |args| {
            panic!("Unwinding!");
        }),
        native_comp_new("_Unwind_Backtrace", |args| {
            Value::from(0u32)
        }),
    ]);
    result.append(&mut overflow_binop!("uadd", "sadd", wrapping_add, checked_add));
    result.append(&mut overflow_binop!("umul", "smul", wrapping_mul, checked_mul));
    result.append(&mut overflow_binop!("usub", "ssub", wrapping_sub, checked_sub));
    result
}
