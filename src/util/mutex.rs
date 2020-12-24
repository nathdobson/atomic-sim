use std::cell::RefCell;
use std::rc::Rc;
use smallvec::alloc::collections::VecDeque;
use std::task::Waker;
use futures::channel::oneshot::{Sender, channel, Canceled};
use std::mem;
use futures::executor::{block_on, LocalPool};
use futures::task::{SpawnExt, LocalSpawnExt};
use std::thread::spawn;
use futures::FutureExt;

pub struct MutexInner {
    locked: bool,
    queue: VecDeque<Sender<!>>,
}

#[derive(Clone)]
pub struct Mutex(Rc<RefCell<MutexInner>>);

pub struct CondvarInner {
    queue: VecDeque<(Mutex, Sender<!>)>,
}

#[derive(Clone)]
pub struct Condvar(Rc<RefCell<CondvarInner>>);

impl Mutex {
    pub fn new() -> Self {
        Mutex(Rc::new(RefCell::new(MutexInner {
            locked: false,
            queue: VecDeque::new(),
        })))
    }
    pub fn try_lock(&self) -> bool {
        let mut this = self.0.borrow_mut();
        if this.locked {
            false
        } else {
            this.locked = true;
            true
        }
    }
    pub async fn lock(&self) {
        let mut this = self.0.borrow_mut();
        if this.locked {
            let (sender, receiver) = channel();
            this.queue.push_back(sender);
            mem::drop(this);
            let Cancelled = receiver.await.unwrap_err();
        } else {
            this.locked = true;
        }
    }
    pub fn unlock(&self) {
        let mut this = self.0.borrow_mut();
        assert!(this.locked);
        if this.queue.pop_front().is_none() {
            this.locked = false;
        }
    }
    fn notify_impl(&self, sender: Sender<!>) {
        let mut this = self.0.borrow_mut();
        if this.locked {
            this.queue.push_back(sender);
        } else {
            mem::drop(sender);
        }
    }
}

impl Condvar {
    pub fn new() -> Self {
        Condvar(Rc::new(RefCell::new(CondvarInner {
            queue: VecDeque::new(),
        })))
    }
    pub async fn wait(&self, mutex: Mutex) {
        mutex.unlock();
        let mut this = self.0.borrow_mut();
        let (sender, receiver) = channel();
        this.queue.push_back((mutex, sender));
        mem::drop(this);
        let Canceled = receiver.await.unwrap_err();
    }
    pub fn notify(&self) {
        let mut this = self.0.borrow_mut();
        if let Some((m, s)) = this.queue.pop_front() {
            m.notify_impl(s);
        }
    }
    pub fn notify_all(&self) {
        let mut this = self.0.borrow_mut();
        for (m, s) in this.queue.drain(..) {
            m.notify_impl(s);
        }
    }
}

#[cfg(test)]
mod test {
    use std::future::Future;
    use std::sync::Mutex as SyncMutex;
    use futures::{
        future::FutureExt, // for `.fuse()`
        pin_mut,
        select,
    };
    use futures::executor::LocalPool;
    use futures::task::{SpawnExt, LocalSpawnExt};
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;
    use futures::future::BoxFuture;
    use crate::util::mutex::{Mutex, Condvar};

    struct TesterState {
        next: usize,
        set: HashSet<usize>,
    }

    struct Token;

    struct Tester {
        pool: LocalPool,
        state: Rc<SyncMutex<TesterState>>,
    }

    impl Tester {
        fn new() -> Self {
            Tester {
                pool: LocalPool::new(),
                state: Rc::new(SyncMutex::new(TesterState { next: 0, set: HashSet::new() })),
            }
        }
        fn add(&self, x: impl Future + 'static) -> usize {
            let key;
            {
                let mut lock = self.state.lock().unwrap();
                key = lock.next;
                lock.next += 1;
            };
            let state = self.state.clone();
            self.pool.spawner().spawn_local(async move {
                x.await;
                state.lock().unwrap().set.insert(key);
            }).unwrap();
            key
        }
        fn run_and_expect(&mut self, ks: &[usize]) {
            self.pool.run_until_stalled();
            let mut lock = self.state.lock().unwrap();
            assert_eq!(lock.set, ks.iter().cloned().collect::<HashSet<_>>());
            lock.set.clear();
        }
    }

    #[test]
    fn test_mutex() {
        let mutex = Mutex::new();
        let mut tester = Tester::new();
        let l1 = tester.add({
            let mutex = mutex.clone();
            async move { mutex.lock().await }
        });
        let l2 = tester.add({
            let mutex = mutex.clone();
            async move { mutex.lock().await }
        });
        tester.run_and_expect(&[l1]);
        mutex.unlock();
        tester.run_and_expect(&[l2]);
    }

    #[test]
    fn test_condvar() {
        let mutex = Mutex::new();
        let condvar = Condvar::new();
        let mut tester = Tester::new();
        let l1 = tester.add({
            let mutex = mutex.clone();
            let condvar = condvar.clone();
            async move {
                println!("A1");
                mutex.lock().await;
                println!("A2");
                condvar.wait(mutex.clone()).await;
                println!("A3");
                mutex.unlock();
                println!("A4");
            }
        });
        tester.run_and_expect(&[]);
        let l2 = tester.add({
            let mutex = mutex.clone();
            let condvar = condvar.clone();
            async move {
                println!("B1");
                mutex.lock().await;
                println!("B2");
                condvar.notify();
                println!("B3");
                mutex.unlock();
                println!("B4");
            }
        });
        tester.run_and_expect(&[l1, l2]);
    }
}