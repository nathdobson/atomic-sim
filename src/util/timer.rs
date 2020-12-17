#[cfg(feature = "timer")]
pub use enabled::*;
#[cfg(feature = "timer")]
pub use enabled::Timer;
#[cfg(feature = "timer")]
pub use enabled::TimerFuture;

#[cfg(not(feature = "timer"))]
pub use disabled::*;

#[cfg(feature = "timer")]
mod enabled {
    use lazy_static::lazy_static;
    use std::time::Instant;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::time::Duration;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::future::Future;
    use std::task::{Context, Poll};
    use std::pin::Pin;

    lazy_static! {
        pub static ref TOTALS: Mutex<HashMap<&'static str, &'static AtomicU64>> = Mutex::new(HashMap::new());
    }

    pub struct Timer {
        pub name: &'static str,
        pub counter: &'static AtomicU64,
        pub start: Instant,
    }

    pub struct TimerFuture<F: Future> {
        pub name: &'static str,
        pub counter: &'static AtomicU64,
        pub fut: F,
    }

    #[macro_export]
    macro_rules! timer {
        ($name: literal) => {
            let _timer = {
                use std::sync::atomic::{AtomicU64, Ordering};
                use $crate::timer::Timer;
                use std::time::Instant;
                static COUNTER: AtomicU64 = AtomicU64::new(0);
                Timer {
                    name: $name,
                    counter: &COUNTER,
                    start: Instant::now(),
                }
            };
        }
    }

    #[macro_export]
    macro_rules! async_timer {
        ($name: literal, $fut: expr) => {
            {
                use std::sync::atomic::{AtomicU64, Ordering};
                use $crate::timer::Timer;
                use $crate::timer::TimerFuture;
                use std::time::Instant;
                static COUNTER: AtomicU64 = AtomicU64::new(0);
                TimerFuture {
                    name: $name,
                    counter: &COUNTER,
                    fut: $fut,
                }
            }
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

    impl<F: Future> Future for TimerFuture<F> {
        type Output = F::Output;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let _timer = Timer {
                name: self.name,
                counter: self.counter,
                start: Instant::now(),
            };
            unsafe { Pin::new_unchecked(&mut self.get_unchecked_mut().fut) }.poll(cx)
        }
    }

    pub fn dump_trace() {
        for (x, y) in TOTALS.lock().unwrap().iter() {
            println!("{}: {:?}", x, Duration::from_nanos(y.load(Ordering::Relaxed)));
        }
    }
}

#[cfg(not(feature = "timer"))]
mod disabled {
    #[macro_export]
    macro_rules! timer {
        ($name: literal) => {}
    }

    #[macro_export]
    macro_rules! async_timer {
        ($name: literal, $fut: expr) => { $fut }
    }

    pub fn dump_trace() {
    }
}
