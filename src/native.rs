use std::cmp::Ordering;
use crate::layout::Layout;
use crate::value::Value;
use crate::function::{Func, native_comp_new, native_exec_new};
use crate::data::{Thunk, ComputeCtx};
use std::rc::Rc;
use std::marker::PhantomData;
use crate::backtrace::BacktraceFrame;
use std::convert::{TryInto, TryFrom};
use crate::flow::FlowCtx;
use crate::layout::Packing;
use crate::function::NativeBomb;
use crate::util::future::FutureExt;
use crate::thread::ThreadId;
use llvm_ir::instruction::MemoryOrdering;

const UNLOCKED: u64 = 0x32AAABA7;
const LOCKED: u64 = 1;

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
            &format!("llvm.{}.with.overflow.{}", $op, $ty),
            move |comp, [x,y]| {
                let (x, y) = (x.$unwrap(), y.$unwrap());
                let types = comp.process.types();
                let v1 = Value::from(x.$wrapping(y));
                let v2 = Value::from(x.$checked(y).is_none());
                let class = types.struc(vec![types.int(v1.bits()), types.int(v2.bits())], false);
                Value::aggregate(&class, [v1, v2].iter().cloned())
            }
        )
    }
}

macro_rules! sat_binop {
    ($uop:expr, $sop:expr, $method:ident) => {
        vec![
            sat_binop!($uop, $method, "i8", unwrap_u8),
            sat_binop!($uop, $method, "i16", unwrap_u16),
            sat_binop!($uop, $method, "i32", unwrap_u32),
            sat_binop!($uop, $method, "i64", unwrap_u64),
            sat_binop!($uop, $method, "i128", unwrap_u128),
            sat_binop!($sop, $method, "i8", unwrap_i8),
            sat_binop!($sop, $method, "i16", unwrap_i16),
            sat_binop!($sop, $method, "i32", unwrap_i32),
            sat_binop!($sop, $method, "i64", unwrap_i64),
            sat_binop!($sop, $method, "i128", unwrap_i128)
        ]
    };
    ($op:expr, $method:ident, $ty:expr, $unwrap:ident) => {
        native_comp_new(
            &format!("llvm.{}.{}", $op, $ty),
            move |comp, [x,y]| {
                let (x, y) = (x.$unwrap(), y.$unwrap());
                Value::from(x.$method(y))
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

async fn lock(flow: &FlowCtx, m: &Value) {
    loop {
        let output = flow.cmpxchg_u64(
            m,
            UNLOCKED,
            LOCKED,
            MemoryOrdering::Acquire,
            MemoryOrdering::Monotonic).await;
        if output == (UNLOCKED, true) {
            break;
        } else {
            assert_eq!(output, (LOCKED, false));
            flow.process().yield_scheduler();
        }
    }
}

async fn unlock(flow: &FlowCtx, m: &Value) {
    assert_eq!(flow.cmpxchg_u64(
        m,
        LOCKED,
        UNLOCKED,
        MemoryOrdering::Release,
        MemoryOrdering::Monotonic).await,
               (LOCKED, true));
}

pub fn builtins() -> Vec<Rc<dyn 'static + Func>> {
    let mut result: Vec<Rc<dyn 'static + Func>> = vec![];
    result.append(&mut vec![
        native_comp_new("llvm.dbg.declare", |comp, [_, _, _]| {
            Value::from(())
        }),
        native_comp_new("llvm.dbg.value", |comp, [_, _, _]| {
            Value::from(())
        }),
        native_comp_new("signal", |comp, [_, _]| {
            comp.process.null()
        }),
        native_comp_new("sysconf", |comp, [flag]| {
            Value::from(match flag.unwrap_u32() {
                29 => unsafe { libc::sysconf(libc::_SC_PAGESIZE) },
                x => todo!("{:?}", x),
            } as u64)
        }),
        native_comp_new("pthread_rwlock_rdlock", |comp, [m]| {
            println!("pthread_rwlock_rdlock({:?})", m);
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_exec_new("pthread_rwlock_unlock", |flow, [m]| async move {
            println!("pthread_rwlock_unlock({:?})", m);
            //TODO synchronize lock
            Value::new(32, 0)
        }.boxed_local()),
        native_exec_new("pthread_mutex_trylock", |flow, [m]| async move {
            let result = flow.cmpxchg_u64(
                &m,
                UNLOCKED,
                LOCKED,
                MemoryOrdering::Acquire,
                MemoryOrdering::Monotonic).await;
            if result.1 {
                Value::from(0u32)
            } else {
                Value::from(16u32)
            }
        }.boxed_local()),
        native_exec_new("pthread_mutex_lock", |flow, [m]| async move {
            lock(flow, &m).await;
            Value::from(0u32)
        }.boxed_local()),
        native_exec_new("pthread_mutex_unlock", |flow, [m]| async move {
            unlock(flow, &m).await;
            Value::from(0u32)
        }.boxed_local()),
        native_comp_new("pthread_mutexattr_init", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutexattr_settype", |comp, [_, _]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_exec_new("pthread_mutex_init", |flow, [m, _]| async move {
            flow.store(&m, &Value::from(UNLOCKED)).await;
            Value::new(32, 0)
        }.boxed_local()),
        native_comp_new("pthread_mutexattr_destroy", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_mutex_destroy", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_cond_broadcast", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_cond_destroy", |comp, [_]| {
            //TODO synchronize lock
            Value::new(32, 0)
        }),
        native_comp_new("pthread_self", |comp, []| {
            comp.process.null()
        }),
        native_comp_new("pthread_get_stackaddr_np", |comp, [_]| {
            comp.process.value_from_address(0)
        }),
        native_comp_new("pthread_get_stacksize_np", |comp, [_]| {
            comp.process.value_from_address(0)
        }),
        native_exec_new("pthread_attr_init", |flow, [attr]| async move {
            let value = [0xFFu8; 64];
            flow.store(&attr, &Value::from_bytes_exact(&value)).await;
            Value::from(0u32)
        }.boxed_local()),
        native_comp_new("pthread_attr_setstacksize", |comp, [attr, stacksize]| {
            Value::from(0u32)
        }),
        native_exec_new("pthread_create", |flow, [thread, attr, start_routine, arg]| async move {
            flow.store(&thread, &Value::from(flow.process().add_thread(&start_routine, &[arg]).0)).await;
            Value::from(0u32)
        }.boxed_local()),
        native_comp_new("pthread_attr_destroy", |comp, [attr]| {
            Value::from(0u32)
        }),
        native_exec_new("pthread_join", |flow, [thread, retval]| async move {
            assert_eq!(retval.as_u64(), 0);
            let thread = ThreadId(thread.unwrap_u64() as usize);
            while flow.process().thread(thread).is_some() {
                flow.constant(Value::from(())).await.await;
            }
            Value::from(0u32)
        }.boxed_local()),
        native_exec_new("pthread_cond_wait", |flow, [cond, mutex]| async move {
            unlock(flow, &mutex).await;
            flow.process().yield_scheduler();
            lock(flow, &mutex).await;
            Value::from(0u32)
        }.boxed_local()),
        native_comp_new("pthread_detach", |comp, [_]| {
            Value::from(0u32)
        }),
        native_comp_new("_tlv_atexit", |comp, [func, objAddr]| {
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
        native_exec_new("llvm.memcpy.p0i8.p0i8.i64", |flow, [dst, src, cnt, _]| async move {
            flow.memcpy(&dst, &src, cnt.as_u64()).await;
            Value::from(())
        }.boxed_local()),
        native_exec_new("llvm.memmove.p0i8.p0i8.i64", |flow, [dst, src, cnt, _]| async move {
            flow.memcpy(&dst, &src, cnt.as_u64()).await;
            Value::from(())
        }.boxed_local()),
        native_comp_new("llvm.ctpop.i64", |comp, [arg]| {
            Value::from(arg.unwrap_u64().count_ones() as u64)
        }),
        native_comp_new("llvm.assume", |comp, [arg]| {
            assert!(arg.unwrap_bool());
            Value::from(())
        }),
        native_exec_new("write", |flow, [fd, buf, len]| async move {
            let value = if len.as_u64() > 0 {
                flow.load(&buf, Layout::from_bytes(len.as_u64(), 1)).await
            } else {
                Value::from(())
            };
            let value = value.as_bytes();
            let string = String::from_utf8(value.to_vec()).unwrap();
            print!("{}", string);
            len
        }.boxed_local()),
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
            comp.process.value_from_address(addr)
        }),
        native_comp_new("mprotect", |comp, [addr, length, prot]| {
            let addr = addr.as_u64();
            let length = length.as_u64();
            let prot = prot.unwrap_u32();
            Value::from(0u32)
        }),
        native_exec_new("llvm.memset.p0i8.i64", |flow, [addr, val, len, _]| async move {
            flow.store(&addr,
                       &Value::from_bytes_exact(&vec![val.unwrap_u8(); len.as_u64() as usize]),
            ).await;
            Value::from(())
        }.boxed_local()),
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
        native_exec_new("__rust_realloc", |flow, [ptr, old_size, align, new_size]| async move {
            flow.realloc(&ptr,
                         Layout::from_bytes(old_size.as_u64(), align.as_u64()),
                         Layout::from_bytes(new_size.as_u64(), align.as_u64())).await
        }.boxed_local()),
        native_comp_new("__rust_dealloc", |comp, [ptr, size, align]| {
            comp.process.free(comp.threadid, &ptr);
            Value::from(())
        }),
        native_exec_new("memchr", |flow, [ptr, value, num]| async move {
            for i in 0..num.as_u64() {
                let ptr = flow.process().value_from_address(ptr.as_u64() + i);
                let v = flow.load(&ptr, Layout::from_bytes(1, 1)).await;
                if v.unwrap_u8() as u32 == value.unwrap_u32() {
                    return ptr;
                }
            }
            flow.process().value_from_address(0)
        }.boxed_local()),
        native_exec_new("strlen", |flow, [str]| async move {
            flow.strlen(&str).await
        }.boxed_local()),
        native_exec_new("memcmp", |flow, [ptr1, ptr2, num]| async move {
            for i in 0..num.as_u64() {
                let ptr1 = flow.process().value_from_address(ptr1.as_u64() + i);
                let ptr2 = flow.process().value_from_address(ptr2.as_u64() + i);
                let v1 = flow.load(&ptr1, Layout::from_bytes(1, 1)).await;
                let v2 = flow.load(&ptr2, Layout::from_bytes(1, 1)).await;
                match v1.unwrap_u8().cmp(&v2.unwrap_u8()) {
                    Ordering::Less => return Value::from(-1i32),
                    Ordering::Equal => {}
                    Ordering::Greater => return Value::from(1i32),
                }
            }
            Value::from(0i32)
        }.boxed_local()),
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
                _ => flow.process().value_from_address(0)
            }
        }.boxed_local()),
        native_exec_new("getcwd", |flow, [buf, size]| async move {
            if buf.as_u64() != 0 {
                if size.as_u64() != 0 {
                    flow.store(&buf, &Value::from(0u8)).await;
                }
                buf
            } else {
                let layout = Layout::of_int(8);
                let res = flow.alloc(layout).await;
                flow.store(&res, &Value::from(0u8)).await;
                res
            }
        }.boxed_local()),
        native_comp_new("_Unwind_RaiseException", |comp, [object]| {
            panic!("Unwinding!");
        }),
        native_exec_new(
            "_Unwind_Backtrace",
            |flow, [trace, trace_argument]| async move {
                for bt in flow.backtrace().iter() {
                    let bctx = flow.process().value_from_address(bt.ip() + 1);
                    let ret = flow.invoke(&trace, &[&bctx, &trace_argument]).await;
                }
                Value::from(0u32)
            }.boxed_local()),
        native_comp_new("_Unwind_GetTextRelBase", |comp, [_]| {
            comp.process.null()
        }),
        native_comp_new("_Unwind_GetDataRelBase", |comp, [_]| {
            comp.process.null()
        }),
        native_comp_new("_Unwind_GetLanguageSpecificData", |comp, [_, _]| {
            comp.process.null()
        }),
        native_comp_new("_Unwind_GetIPInfo", |comp, [_, _]| {
            comp.process.null()
        }),
        native_comp_new("_Unwind_GetRegionStart", |comp, [_, _]| {
            comp.process.null()
        }),
        native_comp_new("_Unwind_SetGR", |comp, [_, _, _]| {
            Value::from(())
        }),
        native_comp_new("_Unwind_SetIP", |comp, [_, _, _]| {
            Value::from(())
        }),
        native_comp_new("explode", |comp, []| {
            panic!();
        }),
        native_exec_new(
            "_Unwind_GetIP",
            |flow, [bctx]| async move {
                bctx
            }.boxed_local()),
        native_exec_new(
            "_NSGetExecutablePath",
            |flow, [buf, len_ptr]| async move {
                let len = flow.load(&len_ptr, Layout::of_int(32)).await;
                let filename = b"unknown.rs\0";
                if len.as_u64() < filename.len() as u64 {
                    return Value::from(0u32);
                }
                flow.store(&buf, &Value::from_bytes_exact(filename)).await;
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
                let zero = flow.process().null();
                let one = flow.process().value_from_address(1);
                let name = flow.string(&format!("{:?}", flow.process().reverse_lookup(&addr))).await;
                flow.invoke(&cb, &[&data, &addr, &name, &addr, &one]).await.unwrap();
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
            flow.process().value_from_address(flow.process().lookup(None, &name).global().unwrap())
        }.boxed_local()),
        native_exec_new(
            "__rdos_backtrace_pcinfo",
            |flow, [state, pc, cb, error, data]| async move {
                //println!("Calling __rdos_backtrace_pcinfo({:?}, {:?}, {:?}, {:?}, {:?})", state, pc, cb, error, data);
                let null = flow.process().null();
                let symbol = flow.process().reverse_lookup(&pc);
                let fun = flow.process().reverse_lookup_fun(&pc).unwrap();
                let symbol_name = flow.string(&format!("{:?}", symbol)).await;
                let debugloc = fun.debugloc();
                let (filename, line) = if let Some(debugloc) = debugloc {
                    (debugloc.filename.as_str(), debugloc.line)
                } else {
                    ("", 0)
                };
                let filename = flow.string(filename).await;
                flow.invoke(&cb, &[&data, &pc, &filename, &Value::from(line), &symbol_name]).await.unwrap();
                Value::from(0u32)
            }.boxed_local()),
        native_exec_new("getentropy", |flow, [buf, len]| async move {
            let data = Value::from_bytes_exact(&vec![4/*chosen by fair dice roll.*/; len.as_u64() as usize]);
            flow.store(&buf, &data).await;
            Value::from(0u32)
        }.boxed_local()),
        native_comp_new("strerror_r", |comp, [errnum, buf, len]| {
            Value::from(-1i32)
        }),
    ]);
    result.append(&mut overflow_binop!("uadd", "sadd", wrapping_add, checked_add));
    result.append(&mut overflow_binop!("umul", "smul", wrapping_mul, checked_mul));
    result.append(&mut overflow_binop!("usub", "ssub", wrapping_sub, checked_sub));
    result.append(&mut sat_binop!("usub.sat", "ssub.sat", saturating_sub));
    result.append(&mut unop!("cttz", trailing_zeros));
    result.append(&mut unop!("ctlz", leading_zeros));
    for name in &[
        "_NSGetArgc",
        "_NSGetArgv",
        "_NSGetEnviron",
        "__error",
        "__rust_alloc_zeroed",
        "_exit",
        "abort",
        "accept",
        "bind",
        "calloc",
        "chdir",
        "chmod",
        "close$NOCANCEL",
        "closedir",
        "connect",
        "copyfile_state_alloc",
        "copyfile_state_free",
        "copyfile_state_get",
        "dup2",
        "execvp",
        "exit",
        "fchmod",
        "fcntl",
        "fcopyfile",
        "fork",
        "free",
        "freeaddrinfo",
        "fstat$INODE64",
        "ftruncate",
        "gai_strerror",
        "getaddrinfo",
        "getpeername",
        "getpid",
        "getppid",
        "getpwuid_r",
        "getsockname",
        "getsockopt",
        "gettimeofday",
        "getuid",
        "ioctl",
        "kill",
        "link",
        "listen",
        "llvm.bswap.i128",
        "llvm.bswap.i16",
        "llvm.bswap.i32",
        "llvm.bswap.v8i16",
        "llvm.ctpop.i32",
        "llvm.fshr.i32",
        "llvm.uadd.sat.i64",
        "lseek",
        "lstat$INODE64",
        "mach_absolute_time",
        "mach_timebase_info",
        "malloc",
        "mkdir",
        "munmap",
        "nanosleep",
        "opendir$INODE64",
        "pipe",
        "poll",
        "posix_memalign",
        "posix_spawn_file_actions_adddup2",
        "posix_spawn_file_actions_destroy",
        "posix_spawn_file_actions_init",
        "posix_spawnattr_destroy",
        "posix_spawnattr_init",
        "posix_spawnattr_setflags",
        "posix_spawnattr_setsigdefault",
        "posix_spawnattr_setsigmask",
        "posix_spawnp",
        "pread",
        "pthread_cond_signal",
        "pthread_cond_timedwait",
        "pthread_key_create",
        "pthread_key_delete",
        "pthread_rwlock_wrlock",
        "pthread_setname_np",
        "pthread_sigmask",
        "pwrite",
        "read",
        "readdir_r$INODE64",
        "readlink",
        "readv",
        "realloc",
        "realpath$DARWIN_EXTSN",
        "recv",
        "recvfrom",
        "rename",
        "rmdir",
        "sched_yield",
        "send",
        "sendto",
        "setenv",
        "setgid",
        "setgroups",
        "setsockopt",
        "setuid",
        "shutdown",
        "sigaddset",
        "sigemptyset",
        "socket",
        "socketpair",
        "stat$INODE64",
        "symlink",
        "unlink",
        "unsetenv",
        "waitpid",
        "writev",
    ] {
        result.push(Rc::new(NativeBomb { name: name.to_string() }));
    }
    result
}

