use std::cell::RefCell;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::ptr::null_mut;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::util::future::{FutureExt, LocalBoxFuture};

struct Frame<T, F: Future<Output=T> + ?Sized> {
    output: Option<T>,
    future: F,
}

type FramePtr = *mut dyn Future<Output=()>;

struct FrameWaiter<'a, T>(*mut Frame<T, dyn 'a + Future<Output=T>>);

struct RecursorInner {
    stack: Vec<FramePtr>,
}

#[derive(Clone)]
pub struct Recursor(Rc<RefCell<RecursorInner>>);

impl<T, F: Future<Output=T>> Unpin for Frame<T, F> {}

impl<T, F: Future<Output=T>> Future for Frame<T, F> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        assert!(self.output.is_none());
        if let Poll::Ready(output) = unsafe { Pin::new_unchecked(&mut self.future) }.poll(cx) {
            self.output = Some(output);
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl<'a, T> Future for FrameWaiter<'a, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            if let Some(result) = (&mut *self.0).output.take() {
                mem::drop(Box::from_raw(self.0));
                self.0 = &mut Frame { output: None, future: async { panic!() } };
                Poll::Ready(result)
            } else {
                Poll::Pending
            }
        }
    }
}


impl Recursor {
    pub fn new() -> Recursor {
        Recursor(Rc::new(RefCell::new(RecursorInner { stack: vec![] })))
    }
    pub fn spawn<'a, T: 'a, F: 'a + Future<Output=T>>(&self, fut: F) -> impl 'a + Future<Output=T> {
        let frame: *mut Frame<T, F> = Box::into_raw(Box::new(Frame { output: None, future: fut }));
        unsafe {
            self.push(
                mem::transmute::<
                    *mut (dyn 'a + Future<Output=()>),
                    *mut (dyn 'static + Future<Output=()>)>(frame));
        }
        FrameWaiter(frame)
    }
    fn push(&self, frame: FramePtr) {
        self.0.borrow_mut().stack.push(frame);
    }
    fn pop(&self) -> Option<FramePtr> {
        self.0.borrow_mut().stack.pop()
    }
    fn peek(&self) -> Option<FramePtr> {
        self.0.borrow().stack.last().cloned()
    }
}

impl Future for Recursor {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            while let Some(top) = self.peek() {
                if let Poll::Ready(()) = Pin::new_unchecked(&mut *top).poll(cx) {
                    let top2 = self.pop().unwrap();
                    assert_eq!(top, top2);
                    continue;
                } else {
                    let top2 = self.peek().unwrap();
                    if top == top2 {
                        return Poll::Pending;
                    } else {
                        continue;
                    }
                }
            }
        }
        Poll::Ready(())
    }
}

#[test]
fn test_recursor() {
    fn fibonacci_rec<'a>(rec: &'a Recursor, n: usize) -> LocalBoxFuture<'a, usize> {
        rec.spawn(async move {
            if n == 0 {
                0
            } else if n == 1 {
                1
            } else {
                fibonacci_rec(rec, n - 1).await + fibonacci_rec(rec, n - 2).await
            }
        }).boxed_local()
    }

    fn fibonacci(n: usize) -> usize {
        use futures::executor::block_on;
        block_on(async {
            let recursor = Recursor::new();
            let recursor2 = recursor.clone();
            let one = fibonacci_rec(&recursor2, n);
            recursor.await;
            one.await
        })
    }
    assert_eq!(0, fibonacci(0));
    assert_eq!(1, fibonacci(1));
    assert_eq!(1, fibonacci(2));
    assert_eq!(2, fibonacci(3));
}
