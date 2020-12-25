use std::cmp::Ordering;
use std::rc::Rc;

use crate::data::ComputeCtx;
use crate::flow::FlowCtx;
use crate::function::Func;
use crate::layout::Layout;
use crate::native::{Addr, native_bomb};
use crate::native_comp;
use crate::native_fn;
use crate::util::future::FutureExt;
use crate::value::Value;

fn signal(_: &ComputeCtx, _: (u32, Addr)) -> Addr {
    Addr(0)
}

fn sysconf(_: &ComputeCtx, (flag, ): (u32, )) -> u64 {
    (match flag {
        29 => unsafe { libc::sysconf(libc::_SC_PAGESIZE) }
        _ => todo!("{:?}", flag),
    }) as u64
}

async fn write(flow: &FlowCtx, (fd, Addr(buf), Addr(len)): (u32, Addr, Addr)) -> Addr {
    let value = if len > 0 {
        flow.load(buf, Layout::from_bytes(len, 1)).await
    } else {
        Value::from(())
    };
    let value = value.as_bytes();
    let string = String::from_utf8(value.to_vec()).unwrap();
    print!("{}", string);
    Addr(len)
}

async fn mmap(_: &FlowCtx, (addr, length, prot, flags, fd, offset): (Addr, Addr, u32, u32, u32, Addr)) -> Addr {
    addr
}

async fn mprotect(_: &FlowCtx, (addr, length, prot): (Addr, Addr, u32)) -> u32 {
    0
}

fn sigaction(_: &ComputeCtx, (signum, act, oldact): (u32, Addr, Addr)) -> u32 {
    0
}

fn sigaltstack(_: &ComputeCtx, (ss, old_ss): (Addr, Addr)) -> u32 {
    0
}

async fn memchr(flow: &FlowCtx, (Addr(ptr), value, Addr(num)): (Addr, u32, Addr)) -> Addr {
    for i in 0..num {
        let ptr = ptr + i;
        let v = flow.load(ptr, Layout::from_bytes(1, 1)).await;
        if v.unwrap_u8() as u32 == value {
            return Addr(ptr);
        }
    }
    Addr(0)
}

async fn strlen(flow: &FlowCtx, (Addr(str), ): (Addr, )) -> Addr {
    Addr(flow.strlen(str).await)
}

async fn memcmp(flow: &FlowCtx, (ptr1, ptr2, num): (Addr, Addr, Addr)) -> i32 {
    for i in 0..num.0 {
        let ptr1 = ptr1.0 + i;
        let ptr2 = ptr2.0 + i;
        let v1 = flow.load(ptr1, Layout::from_bytes(1, 1)).await;
        let v2 = flow.load(ptr2, Layout::from_bytes(1, 1)).await;
        match v1.unwrap_u8().cmp(&v2.unwrap_u8()) {
            Ordering::Less => return -1,
            Ordering::Equal => {}
            Ordering::Greater => return 1
        }
    }
    0
}

async fn getenv(flow: &FlowCtx, (Addr(name), ): (Addr, )) -> Addr {
    let name = flow.get_string(name).await;
    match name.as_str() {
        "RUST_BACKTRACE" => {
            Addr(flow.string("full").await)
        }
        _ => Addr(0)
    }
}

async fn getcwd(flow: &FlowCtx, (Addr(buf), Addr(size)): (Addr, Addr)) -> Addr {
    if buf != 0 {
        if size != 0 {
            flow.store(buf, &Value::from(0u8)).await;
        }
        Addr(buf)
    } else {
        let layout = Layout::of_int(8);
        let res = flow.alloc(layout).await;
        flow.store(res, &Value::from(0u8)).await;
        Addr(res)
    }
}

fn dladdr(_: &ComputeCtx, (addr, info): (Addr, Addr)) -> u32 {
    1
}

async fn open(flow: &FlowCtx, (Addr(path), flags): (Addr, u32)) -> i32 {
    println!("Calling open({:?}, {:?})", flow.get_string(path).await, flags);
    -1
}

async fn dlsym(flow: &FlowCtx, (Addr(handle), Addr(name)): (Addr, Addr)) -> Addr {
    let name = flow.get_string(name).await;
    Addr(flow.process().symbols.lookup(None, &name).global().unwrap())
}

async fn getentropy(flow: &FlowCtx, (Addr(buf), len): (Addr, Addr)) -> u32 {
    let data = Value::from_bytes_exact(&vec![4/*chosen by fair dice roll.*/; len.0 as usize]);
    flow.store(buf, &data).await;
    0
}

async fn strerror_r(flow: &FlowCtx, (errno, buf, len): (u32, Addr, Addr)) -> i32 {
    -1
}

pub fn builtins() -> Vec<Rc<dyn 'static + Func>> {
    let mut result: Vec<Rc<dyn 'static + Func>> = vec![];
    result.append(&mut vec![
        native_comp!(signal),
        native_comp!(sysconf),
        native_fn!(write),
        native_fn!(mmap),
        native_fn!(mprotect),
        native_comp!(sigaction),
        native_comp!(sigaltstack),
        native_fn!(memchr),
        native_fn!(strlen),
        native_fn!(memcmp),
        native_fn!(getenv),
        native_fn!(getcwd),
        native_comp!(dladdr),
        native_fn!(open),
        native_fn!(dlsym),
        native_fn!(getentropy),
        native_fn!(strerror_r),
    ]);
    for bomb in &[
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
        "writev"
    ] {
        result.push(native_bomb(*bomb));
    }
    result
}