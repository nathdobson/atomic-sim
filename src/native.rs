use crate::memory::Memory;
use crate::value::Value;
use crate::ctx::Ctx;
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::layout::Layout;
use std::iter::repeat;

pub struct NativeFunction {
    pub name: String,
    pub imp: Box<dyn Fn(&Ctx, &mut Memory, &[&Value]) -> Value>,
}

impl Debug for NativeFunction {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFunction")
            .field("name", &self.name)
            .finish()
    }
}


impl NativeFunction {
    pub(crate) fn builtins() -> Vec<NativeFunction> {
        vec![
            NativeFunction {
                name: "llvm.dbg.declare".to_string(),
                imp: Box::new(|_, _, _| {
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "signal".to_string(),
                imp: Box::new(|ctx, _, _| {
                    Value::new(ctx.ptr_bits, 0)
                }),
            },
            NativeFunction {
                name: "sysconf".to_string(),
                imp: Box::new(|ctx, _, args| {
                    Value::from(match args[0].unwrap_u32() {
                        29 => unsafe { libc::sysconf(libc::_SC_PAGESIZE) },
                        x => todo!("{:?}", x),
                    } as u64)
                }),
            },
            NativeFunction {
                name: "pthread_rwlock_rdlock".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_lock".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_unlock".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_unlock".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_init".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_settype".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_init".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutexattr_destroy".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "pthread_mutex_destroy".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::new(32, 0)
                }),
            },
            NativeFunction {
                name: "_tlv_atexit".to_string(),
                imp: Box::new(|ctx, _, _| {
                    //TODO synchronize lock
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "llvm.expect.i1".to_string(),
                imp: Box::new(|_, _, xs| {
                    xs[0].clone()
                }),
            },
            NativeFunction {
                name: "llvm.umul.with.overflow.i64".to_string(),
                imp: Box::new(|_, _, args| {
                    let (x, y) = (args[0].unwrap_u64(), args[1].unwrap_u64());
                    Value::aggregate([
                                         Value::from(x.wrapping_mul(y)),
                                         Value::from(x.checked_mul(y).is_none())
                                     ].iter().cloned(), false)
                }),
            },
            NativeFunction {
                name: "llvm.usub.with.overflow.i64".to_string(),
                imp: Box::new(|_, _, args| {
                    let (x, y) = (args[0].unwrap_u64(), args[1].unwrap_u64());
                    Value::aggregate([
                                         Value::from(x.wrapping_sub(y)),
                                         Value::from(x.checked_sub(y).is_none())
                                     ].iter().cloned(), false)
                }),
            },
            NativeFunction {
                name: "llvm.ssub.with.overflow.i64".to_string(),
                imp: Box::new(|_, _, args| {
                    let (x, y) = (args[0].unwrap_i64(), args[1].unwrap_i64());
                    Value::aggregate([
                                         Value::from(x.wrapping_sub(y)),
                                         Value::from(x.checked_sub(y).is_none())
                                     ].iter().cloned(), false)
                }),
            },
            NativeFunction {
                name: "llvm.sadd.with.overflow.i64".to_string(),
                imp: Box::new(|_, _, args| {
                    let (x, y) = (args[0].unwrap_i64(), args[1].unwrap_i64());
                    Value::aggregate([
                                         Value::from(x.wrapping_add(y)),
                                         Value::from(x.checked_add(y).is_none())
                                     ].iter().cloned(), false)
                }),
            },
            NativeFunction {
                name: "llvm.uadd.with.overflow.i64".to_string(),
                imp: Box::new(|_, _, args| {
                    let (x, y) = (args[0].unwrap_u64(), args[1].unwrap_u64());
                    Value::aggregate([
                                         Value::from(x.wrapping_add(y)),
                                         Value::from(x.checked_add(y).is_none())
                                     ].iter().cloned(), false)
                }),
            },
            NativeFunction {
                name: "llvm.cttz.i64".to_string(),
                imp: Box::new(|_, _, args| {
                    Value::from(args[0].as_u64().trailing_zeros() as u64)
                }),
            },
            NativeFunction {
                name: "llvm.memcpy.p0i8.p0i8.i64".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    memory.memcpy(args[0], args[1], args[2].as_u64());
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "llvm.ctpop.i64".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    Value::from(args[0].unwrap_u64().count_ones() as u64)
                }),
            },
            NativeFunction {
                name: "llvm.assume".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    assert!(args[0].unwrap_bool());
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "write".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    let (fd, buf, len) = (args[0], args[1], args[2].as_u64());
                    let value = if len > 0 {
                        memory.load(
                            &buf, len, None,
                            &ctx.array_of(ctx.int(8), len as usize))
                    } else {
                        Value::aggregate(vec![].into_iter(), false)
                    };
                    let string =
                        String::from_utf8((0..len).map(|i| value[i].unwrap_u8()).collect::<Vec<_>>());
                    println!("write({:?}, {:?}, {:?}) -> {:?}", fd, buf, len, string);
                    ctx.value_from_address(len)
                }),
            },
            NativeFunction {
                name: "llvm.trap".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    panic!("It's a trap!");
                }),
            },
            NativeFunction {
                name: "pthread_self".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    ctx.value_from_address(0)
                }),
            },
            NativeFunction {
                name: "pthread_get_stackaddr_np".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    ctx.value_from_address(0)
                }),
            },
            NativeFunction {
                name: "pthread_get_stacksize_np".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    ctx.value_from_address(0)
                }),
            },
            NativeFunction {
                name: "mmap".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    let (addr, length, prot, flags, fd, offset) =
                        (args[0].as_u64(),
                         args[1].as_u64(),
                         args[2].unwrap_u32(),
                         args[3].unwrap_u32(),
                         args[4].unwrap_u32(),
                         args[5].as_u64());
                    //memory.alloc(Layout::from_size_align(length,ctx.page_size))
                    ctx.value_from_address(addr)
                }),
            },
            NativeFunction {
                name: "mprotect".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    let (addr, length, prot) =
                        (args[0].as_u64(),
                         args[1].as_u64(),
                         args[2].unwrap_u32());
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "llvm.memset.p0i8.i64".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    let (addr, val, len) = (args[0], args[1], args[2].unwrap_u64());
                    memory.store(addr, len, &Value::aggregate(repeat(val.clone()).take(len as usize), false), None);
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "sigaction".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "sigaltstack".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    Value::from(0u32)
                }),
            },
            NativeFunction {
                name: "__rust_alloc".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    memory.alloc(Layout::from_size_align(args[0].as_u64(), args[1].as_u64()))
                }),
            },
            NativeFunction {
                name: "__rust_realloc".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    memory.realloc(args[0],
                                   Layout::from_size_align(args[1].as_u64(), args[2].as_u64()),
                                   Layout::from_size_align(args[3].as_u64(), args[2].as_u64()))
                }),
            },
            NativeFunction {
                name: "__rust_dealloc".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    memory.free(args[0]);
                    Value::from(())
                }),
            },
            NativeFunction {
                name: "memchr".to_string(),
                imp: Box::new(|ctx, memory, args| {
                    for i in 0..args[2].as_u64() {
                        let ptr = ctx.value_from_address(args[0].as_u64() + i);
                        let v = memory.load(&ptr, 1, None, &ctx.int(8));
                        if v.unwrap_u8() as u32 == args[1].unwrap_u32() {
                            return ptr;
                        }
                    }
                    ctx.value_from_address(0)
                }),
            },
        ]
    }
}