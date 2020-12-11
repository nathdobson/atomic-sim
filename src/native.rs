use std::cmp::Ordering;
use crate::layout::Layout;
use crate::value::Value;
use crate::function::{Func};
use crate::data::{Thunk};
use std::rc::Rc;
use futures::future::LocalBoxFuture;
use std::marker::PhantomData;
use futures::FutureExt;
use crate::backtrace::BacktraceFrame;
use std::convert::{TryInto, TryFrom};
use crate::flow::FlowCtx;
use crate::compute::ComputeCtx;
use crate::ctx::EvalCtx;
use crate::layout::Packing;

struct NativeComp<'ctx> {
    name: &'ctx str,
    imp: Rc<dyn 'ctx + for<'comp> Fn(ComputeCtx<'ctx, 'comp>, &'comp [&'comp Value]) -> Value>,
}

struct NativeExec<'ctx> {
    name: &'ctx str,
    imp: Rc<dyn 'ctx + for<'flow> Fn(&'flow FlowCtx<'ctx, 'flow>, &'flow [Value]) -> LocalBoxFuture<'flow, Value>>,
}

impl<'ctx> Func<'ctx> for NativeComp<'ctx> {
    fn name(&self) -> &'ctx str {
        self.name
    }


    fn call_imp<'flow>(&'flow self, flow: &'flow FlowCtx<'ctx, 'flow>, args: &'flow [Thunk<'ctx>]) -> LocalBoxFuture<'flow, Thunk<'ctx>> {
        let imp = self.imp.clone();
        Box::pin(flow.data().thunk(self.name.to_string(), flow.backtrace().clone(), args.to_vec(),
                                   move |comp: ComputeCtx<'ctx, '_>, args| {
                                       imp(comp, args)
                                   }))
    }
}

impl<'ctx> Func<'ctx> for NativeExec<'ctx> {
    fn name(&self) -> &'ctx str {
        self.name
    }


    fn call_imp<'flow>(&'flow self, flow: &'flow FlowCtx<'ctx, 'flow>, args: &'flow [Thunk<'ctx>]) -> LocalBoxFuture<'flow, Thunk<'ctx>> {
        Box::pin(async move {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(arg.clone().await);
            }
            flow.data().constant(flow.backtrace().clone(), (self.imp)(flow, &values).await).await
        })
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
        native_comp_new(
            &**Box::leak(Box::new(format!("llvm.{}.with.overflow.{}", $op, $ty))),
            move |comp, [x,y]| {
                let (x, y) = (x.$unwrap(), y.$unwrap());
                Value::aggregate([
                                     Value::from(x.$wrapping(y)),
                                     Value::from(x.$checked(y).is_none())
                                 ].iter().cloned(), Packing::None)
            }
        )
    }
}

macro_rules! unop {
    ($op:expr, $method:ident) => {
        vec![
            unop!($op, $method, "i8", unwrap_u8, u8),
            unop!($op, $method, "i16", unwrap_u16, u16),
            unop!($op, $method, "i32", unwrap_u32, u32),
            unop!($op, $method, "i64", unwrap_u64, u64),
            unop!($op, $method, "i128", unwrap_u128, u128),
        ]
    };
    ($op:expr, $method:ident, $ty:expr, $unwrap:ident, $cast:ty) => {
        native_comp_new(
            &**Box::leak(Box::new(format!("llvm.{}.{}", $op, $ty))),
            move |comp, [x, _]| {
                Value::from(x.$unwrap().$method() as $cast)
            }
        )
    }
}

pub fn native_comp_new<'ctx, const N: usize>(
    name: &'ctx str,
    imp: impl 'static + for<'comp> Fn(ComputeCtx<'ctx, 'comp>, [Value; N]) -> Value)
    -> Rc<dyn 'ctx + Func<'ctx>>
    where [Value; N]: Default {
    Rc::new(NativeComp {
        name: name,
        imp: Rc::new(move |comp, args| {
            assert_eq!(args.len(), N, "{:?}", name);
            let mut array = <[Value; N]>::default();
            for (p, a) in array.iter_mut().zip(args.iter()) {
                *p = (*a).clone();
            }
            imp(comp, array)
        }),
    })
}

pub fn native_exec_new<'ctx, const N: usize, F>(
    name: &'ctx str,
    imp: F)
    -> Rc<dyn 'ctx + Func<'ctx>>
    where [Value; N]: Default,
          F: 'static + for<'flow> Fn(&'flow FlowCtx<'ctx, 'flow>, [Value; N]) -> LocalBoxFuture<'flow, Value>
{
    let imp = Rc::new(imp);
    Rc::new(NativeExec {
        name,
        imp: Rc::new(move |flow, args| {
            let imp = imp.clone();
            async move {
                assert_eq!(args.len(), N);
                let mut array = <[Value; N]>::default();
                array.clone_from_slice(args);
                imp(flow, array).await
            }
        }.boxed_local()),
    })
}

pub fn builtins<'ctx>() -> Vec<Rc<dyn 'ctx + Func<'ctx>>> {
    let mut result: Vec<Rc<dyn 'ctx + Func<'ctx>>> = vec![];
    result.append(&mut vec![
        native_comp_new("llvm.dbg.declare", |comp, [_, _, _]| {
            Value::from(())
        }),
        native_comp_new("llvm.dbg.value", |comp, [_, _, _]| {
            Value::from(())
        }),
        native_comp_new("signal", |comp, [_, _]| {
            Value::new(comp.ctx().ptr_bits, 0)
        }),
        native_comp_new("sysconf", |comp, [flag]| {
            Value::from(match flag.unwrap_u32() {
                29 => unsafe { libc::sysconf(libc::_SC_PAGESIZE) },
                x => todo!("{:?}", x),
            } as u64)
        }),
        native_comp_new("pthread_rwlock_rdlock", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_rwlock_unlock", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_lock", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_unlock", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_unlock", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutexattr_init", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutexattr_settype", |comp, [_, _]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_init", |comp, [_, _]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutexattr_destroy", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_destroy", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_self", |comp, []| {
            comp.ctx().value_from_address(0)
        }),
        native_comp_new("pthread_get_stackaddr_np", |comp, [_]| {
            comp.ctx().value_from_address(0)
        }),
        native_comp_new("pthread_get_stacksize_np", |comp, [_]| {
            comp.ctx().value_from_address(0)
        }),
        native_comp_new("pthread_attr_init", |comp, [attr]| {
            let value = [0xFFu8; 64];
            comp.process.store(comp.threadid, &attr, &Value::from_bytes(&value, Layout::from_bytes(value.len() as u64, 1)), None);
            Value::from(0u32)
        }),
        native_comp_new("pthread_attr_setstacksize", |comp, [attr, stacksize]| {
            Value::from(0u32)
        }),
        native_comp_new("pthread_create", |comp, [thread, attr, start_routine, arg]| {
            Value::from(0u32)
        }),
        native_comp_new("pthread_attr_destroy", |comp, [attr]| {
            Value::from(0u32)
        }),
        native_comp_new("pthread_join", |comp, [_, _]| {
            Value::from(0u32)
        }),
        native_comp_new("_tlv_atexit", |comp, [func, objAddr]| {
            //TODO synchronize lock
            Value::from(())
        }),
        native_comp_new("llvm.expect.i1", |comp, [arg, _]| {
            arg
        }),
        native_comp_new("llvm.lifetime.start.p0i8", |comp, [_, _]| {
            Value::from(())
        }),
        native_comp_new("llvm.lifetime.end.p0i8", |comp, [_, _]| {
            Value::from(())
        }),
        native_comp_new("llvm.memcpy.p0i8.p0i8.i64", |comp, [dst, src, cnt, _]| {
            comp.process.memcpy(comp.threadid, &dst, &src, cnt.as_u64());
            Value::from(())
        }),
        native_comp_new("llvm.memmove.p0i8.p0i8.i64", |comp, [dst, src, cnt, _]| {
            comp.process.memcpy(comp.threadid, &dst, &src, cnt.as_u64());
            Value::from(())
        }),
        native_comp_new("llvm.ctpop.i64", |comp, [arg]| {
            Value::from(arg.unwrap_u64().count_ones() as u64)
        }),
        native_comp_new("llvm.assume", |comp, [arg]| {
            assert!(arg.unwrap_bool());
            Value::from(())
        }),
        native_comp_new("write", |comp, [fd, buf, len]| {
            let value = if len.as_u64() > 0 {
                comp.process.load(comp.threadid, &buf, Layout::from_bytes(len.as_u64(), 1), None)
            } else {
                Value::from(())
            };
            let value = value.as_bytes();
            let string = String::from_utf8(value.to_vec()).unwrap();
            print!("{}", string);
            len
        }),
        native_comp_new("llvm.trap", |comp, []| {
            panic!("It's a trap!");
        }),
        native_comp_new("mmap", |comp, [addr, length, prot, flags, fd, offset]| {
            let addr = addr.as_u64();
            let length = length.as_u64();
            let prot = prot.unwrap_u32();
            let flags = flags.unwrap_u32();
            let fd = fd.unwrap_u32();
            let offset = offset.as_u64();
            println!("mmap {:?}", addr);
            comp.ctx().value_from_address(addr)
        }),
        native_comp_new("mprotect", |comp, [addr, length, prot]| {
            let addr = addr.as_u64();
            let length = length.as_u64();
            let prot = prot.unwrap_u32();
            Value::from(0u32)
        }),
        native_comp_new("llvm.memset.p0i8.i64", |comp, [addr, val, len, _]| {
            comp.process.store(comp.threadid, &addr,
                               &Value::from_bytes(&vec![val.unwrap_u8(); len.as_u64() as usize],
                                                  Layout::from_bytes(len.as_u64(), 1)),
                               None);
            Value::from(())
        }),
        native_comp_new("llvm.fshl.i64", |comp, [a, b, s]| {
            Value::from((((a.as_u128() << 64 | b.as_u128()) << s.as_u128()) >> 64) as u64)
        }),
        native_comp_new("sigaction", |comp, [signum, act, oldact]| {
            Value::from(0u32)
        }),
        native_comp_new("sigaltstack", |comp, [ss, old_ss]| {
            Value::from(0u32)
        }),
        native_comp_new("__rust_alloc", |comp, [len, align]| {
            comp.process.alloc(comp.threadid, Layout::from_bytes(len.as_u64(), align.as_u64()))
        }),
        native_comp_new("__rust_realloc", |comp, [ptr, old_size, align, new_size]| {
            comp.process.realloc(comp.threadid, &ptr,
                                 Layout::from_bytes(old_size.as_u64(), align.as_u64()),
                                 Layout::from_bytes(new_size.as_u64(), align.as_u64()))
        }),
        native_comp_new("__rust_dealloc", |comp, [ptr, size, align]| {
            comp.process.free(comp.threadid, &ptr);
            Value::from(())
        }),
        native_comp_new("memchr", |comp, [ptr, value, num]| {
            for i in 0..num.as_u64() {
                let ptr = comp.ctx().value_from_address(ptr.as_u64() + i);
                let v = comp.process.load(comp.threadid, &ptr, Layout::from_bytes(1, 1), None);
                if v.unwrap_u8() as u32 == value.unwrap_u32() {
                    return ptr;
                }
            }
            comp.ctx().value_from_address(0)
        }),
        native_exec_new("strlen", |flow, [str]| async move {
            flow.strlen(&str).await
        }.boxed_local()),
        native_comp_new("memcmp", |comp, [ptr1, ptr2, num]| {
            for i in 0..num.as_u64() {
                let ptr1 = comp.ctx().value_from_address(ptr1.as_u64() + i);
                let ptr2 = comp.ctx().value_from_address(ptr2.as_u64() + i);
                let v1 = comp.process.load(comp.threadid, &ptr1, Layout::from_bytes(1, 1), None);
                let v2 = comp.process.load(comp.threadid, &ptr2, Layout::from_bytes(1, 1), None);
                match v1.unwrap_u8().cmp(&v2.unwrap_u8()) {
                    Ordering::Less => return Value::from(-1i32),
                    Ordering::Equal => {}
                    Ordering::Greater => return Value::from(1i32),
                }
            }
            Value::from(0i32)
        }),
        native_exec_new("getenv", |flow, [name]| async move {
            let mut cstr = vec![];
            for i in name.as_u64().. {
                let c = flow.load(&Value::from(i), Layout::of_int(8)).await.unwrap_u8();
                if c == 0 { break; } else { cstr.push(c) }
            }
            let str = String::from_utf8(cstr).unwrap();
            match str.as_str() {
                "RUST_BACKTRACE" => {
                    flow.string("full").await
                }
                _ => flow.ctx().value_from_address(0)
            }
        }.boxed_local()),
        native_comp_new("getcwd", |comp, [buf, size]| {
            if buf.as_u64() != 0 {
                if size.as_u64() != 0 {
                    comp.process.store(comp.threadid, &buf, &Value::from(0u8), None);
                }
                buf
            } else {
                let layout = Layout::of_int(8);
                let res = comp.process.alloc(comp.threadid, layout);
                comp.process.store(comp.threadid, &res, &Value::from(0u8), None);
                res
            }
        }),
        native_comp_new("_Unwind_RaiseException", |comp, [object]| {
            panic!("Unwinding!");
        }),
        native_exec_new(
            "_Unwind_Backtrace",
            |flow, [trace, trace_argument]| async move {
                for bt in flow.backtrace().iter() {
                    let bctx = flow.ctx().value_from_address(bt.ip + 1);
                    let ret = flow.invoke(&trace, &[&bctx, &trace_argument]).await;
                }
                Value::from(0u32)
            }.boxed_local()),
        native_exec_new(
            "_Unwind_GetIP",
            |flow, [bctx]| async move {
                bctx
            }.boxed_local()),
        native_exec_new(
            "_NSGetExecutablePath",
            |flow, [buf, len_ptr]| async move {
                let len = flow.load(&len_ptr, flow.ctx().layout_of_ptr()).await;
                let filename = b"unknown.rs\0";
                if len.as_u64() < filename.len() as u64 {
                    return Value::from(0u32);
                }
                flow.store(&buf, &Value::from_bytes_unaligned(filename)).await;
                flow.store(&len_ptr, &Value::from(filename.len() as u32)).await;
                Value::from(0u32)
            }.boxed_local()),
        native_exec_new(
            "__rdos_backtrace_create_state",
            |flow, [filename, threaded, error, data]| async move {
                //println!("Calling __rdos_backtrace_create_state({:?},{:?},{:?},{:?})", filename, threaded, error, data);
                Value::from(0xCAFEBABEu64)
            }.boxed_local()),
        native_exec_new(
            "__rdos_backtrace_syminfo",
            |flow, [state, addr, cb, error, data]| async move {
                let zero = flow.ctx().null();
                let one = flow.ctx().value_from_address(1);
                let name = flow.string(&format!("{:?}", flow.ctx().reverse_lookup(&addr))).await;
                flow.invoke(&cb, &[&data, &addr, &name, &addr, &one]).await;
                Value::from(0u32)
            }.boxed_local()),
        native_exec_new(
            "dladdr",
            |flow, [addr, info]| async move {
                println!("Calling dladdr({:?}, {:?})", addr, info);
                Value::from(1u32)
            }.boxed_local()),
        native_exec_new(
            "open",
            |flow, [path, flags]| async move {
                println!("Calling open({:?}, {:?})", flow.get_string(&path).await, flags);
                Value::from(0u32)
            }.boxed_local()),
        native_exec_new("dlsym", |flow, [handle, name]| async move {
            let name = flow.get_string(&name).await;
            flow.ctx().lookup(EvalCtx { module: None }, &name).clone()
        }.boxed_local()),
        native_exec_new(
            "__rdos_backtrace_pcinfo",
            |flow, [state, pc, cb, error, data]| async move {
                //println!("Calling __rdos_backtrace_pcinfo({:?}, {:?}, {:?}, {:?}, {:?})", state, pc, cb, error, data);
                let null = flow.ctx().null();
                let symbol = flow.ctx().reverse_lookup(&pc);
                let fun = flow.ctx().functions.get(&symbol).unwrap();
                let symbol_name = flow.string(&format!("{:?}", symbol)).await;
                let debugloc = fun.debugloc();
                let (filename, line) = if let Some(debugloc) = debugloc {
                    (debugloc.filename.as_str(), debugloc.line)
                } else {
                    ("", 0)
                };
                let filename = flow.string(filename).await;
                flow.invoke(&cb, &[&data, &pc, &filename, &Value::from(line), &symbol_name]).await;
                Value::from(0u32)
            }.boxed_local()),
        native_exec_new("getentropy", |flow, [buf, len]| async move {
            let data = Value::from_bytes_unaligned(&vec![4/*chosen by fair dice roll.*/; len.as_u64() as usize]);
            flow.store(&buf, &data).await;
            Value::from(0u32)
        }.boxed_local()),
    ]);
    result.append(&mut overflow_binop!("uadd", "sadd", wrapping_add, checked_add));
    result.append(&mut overflow_binop!("umul", "smul", wrapping_mul, checked_mul));
    result.append(&mut overflow_binop!("usub", "ssub", wrapping_sub, checked_sub));
    result.append(&mut unop!("cttz", trailing_zeros));
    result.append(&mut unop!("ctlz", leading_zeros));
    result
}

