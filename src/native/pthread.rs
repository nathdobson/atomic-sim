use std::rc::Rc;

use llvm_ir::instruction::MemoryOrdering;

use crate::flow::FlowCtx;
use crate::function::Func;
use crate::native::{Addr, native_bomb};
use crate::native_fn;
use crate::ordering::Ordering;
use crate::thread::{Blocker, ThreadId};
use crate::util::future::pending_once;
use crate::value::Value;

const UNLOCKED: u64 = 0x32AAABA7;
const LOCKED: u64 = 1;

async fn mutex_lock_acquire(flow: &FlowCtx, address: u64) {
    assert_eq!((UNLOCKED, true), flow.cmpxchg_u64(
        address,
        UNLOCKED,
        LOCKED,
        Ordering::Acquire,
        Ordering::Relaxed).await);
}

async fn mutex_lock_release(flow: &FlowCtx, address: u64) {
    assert_eq!((LOCKED, true), flow.cmpxchg_u64(
        address,
        LOCKED,
        UNLOCKED,
        Ordering::Release,
        Ordering::Relaxed).await);
}

async fn pthread_mutex_trylock(flow: &FlowCtx, (Addr(m), ): (Addr, )) -> u32 {
    if flow.process().scheduler().mutex(m).try_lock() {
        mutex_lock_acquire(flow, m).await;
        0
    } else {
        1
    }
}

async fn pthread_mutex_lock(flow: &FlowCtx, (Addr(m), ): (Addr, )) -> u32 {
    flow.process().scheduler().thread(flow.threadid()).unwrap().set_blocker(Blocker::Mutex);
    flow.process().scheduler().mutex(m).lock().await;
    flow.process().scheduler().thread(flow.threadid()).unwrap().set_blocker(Blocker::Unknown);
    mutex_lock_acquire(flow, m).await;
    0
}

async fn pthread_mutex_unlock(flow: &FlowCtx, (Addr(m), ): (Addr, )) -> u32 {
    mutex_lock_release(flow, m).await;
    flow.process().scheduler().mutex(m).unlock();
    0
}

async fn pthread_mutex_init(flow: &FlowCtx, (Addr(m), attr): (Addr, Addr)) -> u32 {
    flow.store(m, &Value::from(UNLOCKED)).await;
    0
}

async fn pthread_cond_wait(flow: &FlowCtx, (Addr(cond), Addr(mutex)): (Addr, Addr)) -> u32 {
    mutex_lock_release(flow, mutex).await;
    flow.process().scheduler().thread(flow.threadid()).unwrap().set_blocker(Blocker::Cond);
    let scheduler = flow.process().scheduler();
    scheduler.condvar(cond).wait(scheduler.mutex(mutex)).await;
    flow.process().scheduler().thread(flow.threadid()).unwrap().set_blocker(Blocker::Unknown);
    mutex_lock_acquire(flow, mutex).await;
    0
}

async fn pthread_rwlock_rdlock(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    panic!("pthread_rwlock_rdlock({:?})", m);
    //0
}

async fn pthread_rwlock_unlock(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    panic!("pthread_rwlock_unlock({:?})", m);
    //TODO
    //0
}

async fn pthread_mutexattr_init(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    //println!("pthread_mutexattr_init");
    //TODO
    0
}

async fn pthread_mutexattr_settype(flow: &FlowCtx, (attr, typ): (Addr, u32)) -> u32 {
    //println!("pthread_mutexattr_settype");
    //TODO
    0
}

async fn pthread_mutexattr_destroy(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    //println!("pthread_mutexattr_destroy");
    //TODO
    0
}

async fn pthread_mutex_destroy(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    //println!("pthread_mutex_destroy");
    //TODO
    0
}

async fn pthread_cond_broadcast(flow: &FlowCtx, (Addr(c), ): (Addr, )) -> u32 {
    flow.process().scheduler.condvar(c).notify_all();
    0
}

async fn pthread_cond_signal(flow: &FlowCtx, (Addr(c), ): (Addr, )) -> u32 {
    flow.process().scheduler.condvar(c).notify();
    0
}

async fn pthread_cond_destroy(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    //println!("pthread_cond_destroy");
    //TODO
    0
}

async fn pthread_self(flow: &FlowCtx, (): ()) -> u64 {
    flow.data().threadid().0 as u64
}

async fn pthread_get_stackaddr_np(flow: &FlowCtx, (m, ): (Addr, )) -> Addr {
    //println!("pthread_get_stackaddr_np");
    Addr(0)
}

async fn pthread_get_stacksize_np(flow: &FlowCtx, (m, ): (Addr, )) -> Addr {
    //println!("pthread_get_stacksize_np");
    Addr(0)
}

async fn pthread_attr_init(flow: &FlowCtx, (Addr(attr), ): (Addr, )) -> u32 {
    //println!("pthread_attr_init");
    let value = [0xFFu8; 64];
    flow.store(attr, &Value::from_bytes_exact(&value)).await;
    0
}

async fn pthread_attr_setstacksize(flow: &FlowCtx, (attr, stacksize): (Addr, Addr)) -> u32 {
    //println!("pthread_attr_setstacksize");
    0
}

async fn pthread_create(
    flow: &FlowCtx,
    (Addr(thread), attr, Addr(start_routine), arg): (Addr, Value, Addr, Value)) -> u32 {
    let threadid = flow.process().add_thread(start_routine, &[arg]);
    flow.store(
        thread,
        &Value::from(threadid.0)).await;
    0
}

async fn pthread_attr_destroy(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    0
}

async fn pthread_join(flow: &FlowCtx, (thread, Addr(retval)): (u64, Addr)) -> u32 {
    let thread = ThreadId(thread as usize);
    let result = flow.process().thread(thread).unwrap().join().await;
    if retval != 0 {
        flow.store(retval, &result).await;
    }
    0
}

async fn pthread_detach(flow: &FlowCtx, (m, ): (u64, )) -> u32 {
    0
}

pub fn builtins() -> Vec<Rc<dyn Func>> {
    let mut result = vec![
        native_fn!(pthread_mutex_trylock),
        native_fn!(pthread_mutex_lock),
        native_fn!(pthread_mutex_unlock),
        native_fn!(pthread_mutex_init),
        native_fn!(pthread_cond_wait),
        native_fn!(pthread_rwlock_rdlock),
        native_fn!(pthread_rwlock_unlock),
        native_fn!(pthread_mutexattr_init),
        native_fn!(pthread_mutexattr_settype),
        native_fn!(pthread_mutexattr_destroy),
        native_fn!(pthread_mutex_destroy),
        native_fn!(pthread_cond_broadcast),
        native_fn!(pthread_cond_destroy),
        native_fn!(pthread_self),
        native_fn!(pthread_get_stackaddr_np),
        native_fn!(pthread_get_stacksize_np),
        native_fn!(pthread_attr_init),
        native_fn!(pthread_attr_setstacksize),
        native_fn!(pthread_create),
        native_fn!(pthread_attr_destroy),
        native_fn!(pthread_join),
        native_fn!(pthread_detach),
        native_fn!(pthread_cond_signal),
    ];
    for bomb in [
        "pthread_cond_timedwait",
        "pthread_key_create",
        "pthread_key_delete",
        "pthread_rwlock_wrlock",
        "pthread_setname_np",
        "pthread_sigmask",
    ].iter() {
        result.push(native_bomb(bomb));
    }
    result
}

