use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::collections::hash_map::Entry;
use std::fmt;
use std::fmt::Debug;
use std::ops::Bound;
use std::rc::Rc;

use futures::executor::{LocalPool, LocalSpawner};
use rand::{Rng, SeedableRng};
use rand::seq::IteratorRandom;
use rand_xorshift::XorShiftRng;
use smallvec::alloc::fmt::Formatter;

use crate::process::Process;
use crate::thread::{Thread, ThreadId};
use crate::util::mutex::{Condvar, Mutex};
use crate::value::Value;
use futures::FutureExt;

#[derive(Debug, Clone)]
pub struct Scheduler(Rc<RefCell<SchedulerInner>>);

pub struct SchedulerInner {
    threads: BTreeMap<ThreadId, Thread>,
    next_threadid: usize,
    mutexes: HashMap<u64, Mutex>,
    condvars: HashMap<u64, Condvar>,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler(Rc::new(RefCell::new(SchedulerInner {
            threads: BTreeMap::new(),
            next_threadid: 0,
            mutexes: HashMap::new(),
            condvars: HashMap::new(),
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
        assert!(this.threads.insert(thread.threadid(), thread).is_none());
    }
    pub fn remove_thread(&self, threadid: &ThreadId) {
        let mut this = self.0.borrow_mut();
        this.threads.remove(threadid);
    }
    pub fn thread(&self, threadid: ThreadId) -> Option<Thread> {
        let this = self.0.borrow();
        this.threads.get(&threadid).cloned()
    }
    pub fn threads(&self) -> Vec<Thread> {
        self.0.borrow().threads.values().cloned().collect()
    }
    pub fn reap_threads(&self) {
        self.0.borrow_mut().threads.retain(|_, t| { t.alive() });
    }
    pub fn cancel(&self) {
        let mut this = self.0.borrow_mut();
        this.threads.clear();
    }
    pub fn mutex(&self, address: u64) -> Mutex {
        self.0.borrow_mut().mutexes.entry(address).or_insert_with(|| Mutex::new()).clone()
    }

    pub fn condvar(&self, address: u64) -> Condvar {
        self.0.borrow_mut().condvars.entry(address).or_insert_with(|| Condvar::new()).clone()
    }
}

impl Debug for SchedulerInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchedulerInner")
            .field("threads", &self.threads)
            .field("next_threadid", &self.next_threadid)
            .finish()
    }
}