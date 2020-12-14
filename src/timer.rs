#[cfg(feature = "timer")]
pub use enabled::*;
pub use enabled::Timer;

#[cfg(not(feature = "timer"))]
pub use disabled::*;

mod enabled {
    use lazy_static::lazy_static;
    use std::time::Instant;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::time::Duration;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    lazy_static! {
        pub static ref TOTALS: Mutex<HashMap<&'static str, &'static AtomicU64>> = Mutex::new(HashMap::new());
    }

    pub struct Timer {
        pub name: &'static str,
        pub counter: &'static AtomicU64,
        pub start: Instant,
    }

    #[macro_export]
    macro_rules! timer {
        ($name: literal) => {
            use std::sync::atomic::{AtomicU64, Ordering};
            use $crate::timer::Timer;
            use std::time::Instant;
            let _timer = {
                static COUNTER: AtomicU64 = AtomicU64::new(0);
                Timer {
                    name: $name,
                    counter: &COUNTER,
                    start: Instant::now(),
                }
            };
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            let delta = Instant::now() - self.start;
            let old = self.counter.fetch_add(delta.as_nanos() as u64, Ordering::Relaxed);
            if old == 0 {
                TOTALS.lock().unwrap().insert(self.name, self.counter);
            }
        }
    }

    pub fn dump_trace() {
        for (x, y) in TOTALS.lock().unwrap().iter() {
            println!("{}: {:?}", x, Duration::from_nanos(y.load(Ordering::Relaxed)));
        }
    }
}

mod disabled {
    macro_rules! timer {
        ($name: literal)=>{}
    }

    pub fn dump_trace() {
        println!("Timer disabled");
    }
}
