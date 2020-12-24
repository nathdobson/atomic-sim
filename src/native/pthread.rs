use std::rc::Rc;
use crate::function::{Func};
use crate::flow::FlowCtx;
use crate::value::Value;
use llvm_ir::instruction::MemoryOrdering;
use crate::native_fn;
use crate::native::{native_bomb, Addr};
use crate::thread::{ThreadId, Blocker};
use crate::util::future::pending_once;
use crate::ordering::Ordering;

const UNLOCKED: u64 = 0x32AAABA7;
const LOCKED: u64 = 1;

async fn mutex_lock_acquire(flow: &FlowCtx, address: &Value) {
    assert_eq!((UNLOCKED, true), flow.cmpxchg_u64(
        address,
        UNLOCKED,
        LOCKED,
        Ordering::Acquire,
        Ordering::Relaxed).await);
}

async fn mutex_lock_release(flow: &FlowCtx, address: &Value) {
    assert_eq!((LOCKED, true), flow.cmpxchg_u64(
        address,
        LOCKED,
        UNLOCKED,
        Ordering::Release,
        Ordering::Relaxed).await);
}

async fn pthread_mutex_trylock(flow: &FlowCtx, (m, ): (Value, )) -> u32 {
    if flow.process().scheduler().mutex(m.as_u64()).try_lock() {
        mutex_lock_acquire(flow, &m).await;
        0
    } else {
        1
    }
}

async fn pthread_mutex_lock(flow: &FlowCtx, (m, ): (Value, )) -> u32 {
    flow.process().scheduler().thread(flow.threadid()).unwrap().set_blocker(Blocker::Mutex);
    flow.process().scheduler().mutex(m.as_u64()).lock().await;
    flow.process().scheduler().thread(flow.threadid()).unwrap().set_blocker(Blocker::Unknown);
    mutex_lock_acquire(flow, &m).await;
    0
}

async fn pthread_mutex_unlock(flow: &FlowCtx, (m, ): (Value, )) -> u32 {
    mutex_lock_release(flow, &m).await;
    flow.process().scheduler().mutex(m.as_u64()).unlock();
    0
}

async fn pthread_mutex_init(flow: &FlowCtx, (m, attr): (Value, Addr)) -> u32 {
    flow.store(&m, &Value::from(UNLOCKED)).await;
    0
}

async fn pthread_cond_wait(flow: &FlowCtx, (cond, mutex): (Value, Value)) -> u32 {
    mutex_lock_release(flow, &mutex).await;
    flow.process().scheduler().thread(flow.threadid()).unwrap().set_blocker(Blocker::Cond);
    let scheduler = flow.process().scheduler();
    scheduler.condvar(cond.as_u64()).wait(scheduler.mutex(mutex.as_u64())).await;
    flow.process().scheduler().thread(flow.threadid()).unwrap().set_blocker(Blocker::Unknown);
    mutex_lock_acquire(flow, &mutex).await;
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

async fn pthread_cond_broadcast(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    todo!();
    //0
}

async fn pthread_cond_signal(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    todo!();
    //0
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

async fn pthread_attr_init(flow: &FlowCtx, (attr, ): (Value, )) -> u32 {
    //println!("pthread_attr_init");
    let value = [0xFFu8; 64];
    flow.store(&attr, &Value::from_bytes_exact(&value)).await;
    0
}

async fn pthread_attr_setstacksize(flow: &FlowCtx, (attr, stacksize): (Addr, Addr)) -> u32 {
    //println!("pthread_attr_setstacksize");
    0
}

async fn pthread_create(flow: &FlowCtx, (thread, attr, start_routine, arg): (Value, Value, Value, Value)) -> u32 {
    let threadid = flow.process().add_thread(&start_routine, &[arg]);
    flow.store(
        &thread,
        &Value::from(threadid.0)).await;
    0
}

async fn pthread_attr_destroy(flow: &FlowCtx, (m, ): (Addr, )) -> u32 {
    0
}

async fn pthread_join(flow: &FlowCtx, (thread, retval): (u64, Addr)) -> u32 {
    assert_eq!(retval.0, 0);
    let thread = ThreadId(thread as usize);
    while flow.process().thread(thread).is_some() {
        flow.constant(Value::from(())).await.await;
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

