use std::future::Future;
use std::task::{Context, Poll, Waker, RawWaker, RawWakerVTable};
use std::pin::Pin;
use std::ptr::null;

pub fn pending_once() -> impl Future {
    struct Fut(bool);
    impl Future for Fut {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.0 {
                Poll::Ready(())
            } else {
                self.0 = true;
                Poll::Pending
            }
        }
    }
    Fut(false)
}

pub type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output=T> + 'a>>;

pub trait FutureExt: Future {
    fn boxed_local<'a>(self) -> LocalBoxFuture<'a, Self::Output>
        where Self: Sized + 'a {
        Box::pin(self)
    }
}

impl<T> FutureExt for T where T: Future {}

static NOOP_TABLE: RawWakerVTable = RawWakerVTable::new(
    |x| noop_raw_waker(),
    |x| {},
    |x| {},
    |x| {},
);

fn noop_raw_waker() -> RawWaker {
    RawWaker::new(null(), &NOOP_TABLE)
}

pub fn noop_waker() -> Waker {
    unsafe { Waker::from_raw(noop_raw_waker()) }
}