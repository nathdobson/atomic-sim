use crate::thread::{ThreadId, Thread};
use std::collections::{VecDeque, BTreeMap, HashSet, HashMap};
use crate::process::Process;
use crate::value::Value;
use std::fmt::Debug;
use smallvec::alloc::fmt::Formatter;
use std::fmt;
use rand_xorshift::XorShiftRng;
use rand::seq::IteratorRandom;
use rand::{Rng, SeedableRng};
use std::collections::hash_map::Entry;
use std::ops::Bound;
use std::rc::Rc;
use std::cell::RefCell;

#[derive(Debug, Default)]
struct MutexState {
    owner: Option<ThreadId>,
    queue: VecDeque<ThreadId>,
}

#[derive(Debug, Default)]
struct CondState {
    mutex: u64,
    queue: VecDeque<ThreadId>,
}

#[derive(Debug, Clone)]
pub struct Scheduler(Rc<RefCell<SchedulerInner>>);

pub struct SchedulerInner {
    threads: BTreeMap<ThreadId, Thread>,
    next_threadid: usize,
    active_thread: Option<(ThreadId, usize)>,
    runnable_threads: HashSet<ThreadId>,
    mutexes: HashMap<u64, MutexState>,
    conds: HashMap<u64, CondState>,
    rng: XorShiftRng,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler(Rc::new(RefCell::new(SchedulerInner {
            threads: BTreeMap::new(),
            next_threadid: 0,
            active_thread: None,
            runnable_threads: HashSet::new(),
            mutexes: HashMap::new(),
            conds: HashMap::new(),
            rng: XorShiftRng::seed_from_u64(0),
        })))
    }
    pub fn new_threadid(&self) -> ThreadId {
        let mut this = self.0.borrow_mut();
        let threadid = ThreadId(this.next_threadid);
        this.next_threadid += 1;
        threadid
    }
    pub fn add_thread(&self, thread: Thread) {
        let mut this = self.0.borrow_mut();
        assert!(this.runnable_threads.insert(thread.threadid()));
        assert!(this.threads.insert(thread.threadid(), thread).is_none());
    }
    pub fn remove_thread(&self, threadid: &ThreadId) {
        let mut this = self.0.borrow_mut();
        this.threads.remove(threadid);
        assert!(this.runnable_threads.remove(&threadid));
        this.active_thread = None;
    }
    pub fn thread(&self, threadid: &ThreadId) -> Option<Thread> {
        let this = self.0.borrow();
        this.threads.get(threadid).cloned()
    }
    pub fn schedule(&self) -> Option<Thread> {
        //println!("{:?}.schedule()", self);
        let mut this = self.0.borrow_mut();
        let this = &mut *this;
        if let Some((t, c)) = this.active_thread {
            assert!(this.threads.contains_key(&t));
            assert!(this.runnable_threads.contains(&t));
        }
        if this.active_thread.is_none() {
            if let Some(t) = this.runnable_threads.iter().choose(&mut this.rng) {
                this.active_thread = Some((*t, this.rng.gen_range(1, 10)));
            }
        }
        this.active_thread.take().map(|(t, c)| {
            if c > 0 {
                this.active_thread = Some((t, c - 1));
            }
            this.threads.get(&t).unwrap().clone()
        })
    }
    pub fn mutex_trylock(&self, threadid: ThreadId, address: u64) -> bool {
        //println!("{:?}.mutex_trylock({:?}, {:?})", self, threadid, address);
        let mut this = self.0.borrow_mut();
        let mutex = this.mutexes.entry(address).or_default();
        if mutex.owner == None {
            mutex.owner = Some(threadid);
            true
        } else {
            false
        }
    }
    pub fn mutex_lock_enter(&self, threadid: ThreadId, address: u64) -> bool {
        //println!("{:?}.mutex_lock_enter({:?}, {:?})", self, threadid, address);
        let mut this = self.0.borrow_mut();
        let this = &mut *this;
        let mutex = this.mutexes.entry(address).or_default();
        if mutex.owner == None {
            mutex.owner = Some(threadid);
            true
        } else {
            this.runnable_threads.remove(&threadid);
            mutex.queue.push_back(threadid);
            this.active_thread = None;
            false
        }
    }
    pub fn mutex_lock_accept(&self, threadid: ThreadId, address: u64) -> bool {
        //println!("{:?}.mutex_lock_accept({:?}, {:?})", self, threadid, address);
        let this = self.0.borrow();
        this.mutexes.get(&address).unwrap().owner == Some(threadid)
    }
    pub fn mutex_unlock(&self, threadid: ThreadId, address: u64) {
        //println!("{:?}.mutex_unlock({:?}, {:?})", self, threadid, address);
        let mut this = self.0.borrow_mut();
        Self::mutex_unlock_inner(&mut *this, threadid, address);
    }
    pub fn mutex_unlock_inner(this: &mut SchedulerInner, threadid: ThreadId, address: u64) {
        //println!("{:?}.mutex_unlock_inner({:?}, {:?})", this, threadid, address);
        let mutex = this.mutexes.entry(address).or_default();
        assert_eq!(mutex.owner, Some(threadid));
        mutex.owner = mutex.queue.pop_front();
        if let Some(woken) = mutex.owner {
            assert!(this.runnable_threads.insert(woken));
        }
    }
    pub fn cond_wait(&self, threadid: ThreadId, cond: u64, mutex: u64) {
        //println!("{:?}.cond_wait({:?}, {:?}, {:?})", self, threadid, cond, mutex);
        let mut this = self.0.borrow_mut();
        let cond = this.conds.entry(cond).or_default();
        cond.mutex = mutex;
        cond.queue.push_back(threadid);
        assert!(this.runnable_threads.remove(&threadid));
        this.active_thread = None;
        Self::mutex_unlock_inner(&mut *this, threadid, mutex);
    }
    pub fn cond_notify(&self, threadid: ThreadId, address: u64, count: Option<usize>) {
        //println!("{:?}.cond_notify({:?}, {:?})", self, threadid, address);
        let mut this = self.0.borrow_mut();
        let this = &mut *this;
        let cond = this.conds.entry(address).or_default();
        let drain = cond.queue.drain(..count.unwrap_or(cond.queue.len()));
        let mutex = this.mutexes.entry(cond.mutex).or_default();
        for popped in drain {
            mutex.queue.push_back(popped);
        }
        if mutex.owner == None {
            mutex.owner = mutex.queue.pop_front();
            if let Some(woken) = mutex.owner {
                assert!(this.runnable_threads.insert(woken));
            }
        }
    }
    pub fn cancel(&self) {
        let mut this = self.0.borrow_mut();
        this.threads.clear();
        this.runnable_threads.clear();
        this.active_thread = None;
    }
}

impl Debug for SchedulerInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchedulerInner")
            .field("threads", &self.threads.keys())
            .field("active_thread", &self.active_thread)
            .field("runnable_threads", &self.runnable_threads)
            .field("mutexes", &self.mutexes)
            .field("conds", &self.conds)
            .field("next_threadid", &self.next_threadid)
            .field("rng", &self.rng)
            .finish()
    }
}