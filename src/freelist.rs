use std::rc::Rc;
use std::cell::RefCell;
use std::ptr::NonNull;
use std::marker::PhantomData;
use std::{ptr, mem};
use std::mem::MaybeUninit;
use std::ops::Deref;

struct FrcInner<T> {
    refcount: usize,
    value: T,
    list: FreeList<T>,
}

struct FreeListInner<T> {
    free: Vec<NonNull<MaybeUninit<FrcInner<T>>>>,
}

pub struct FreeList<T> { inner: Rc<RefCell<FreeListInner<T>>> }

pub struct Frc<T> {
    ptr: NonNull<MaybeUninit<FrcInner<T>>>,
    phantom: PhantomData<MaybeUninit<FrcInner<T>>>,
}

impl<T> FreeList<T> {
    pub fn new() -> FreeList<T> {
        FreeList { inner: Rc::new(RefCell::new(FreeListInner { free: vec![] })) }
    }
    pub fn alloc(&self, value: T) -> Frc<T> {
        unsafe {
            let ptr =
                self.inner.borrow_mut().free
                    .pop()
                    .unwrap_or_else(||
                        NonNull::new(Box::into_raw(Box::new_uninit())).unwrap());
            (*ptr.as_ptr()).write(FrcInner {
                refcount: 0,
                list: self.clone(),
                value,
            });
            Frc { ptr, phantom: PhantomData }
        }
    }
}

impl<T> Frc<T> {
    unsafe fn inner(&self) -> &mut FrcInner<T> {
        &mut *(*self.ptr.as_ptr()).as_mut_ptr()
    }
}

impl<T> Clone for FreeList<T> {
    fn clone(&self) -> Self {
        FreeList { inner: self.inner.clone() }
    }
}

impl<T> Clone for Frc<T> {
    fn clone(&self) -> Self {
        unsafe {
            (*(*self.ptr.as_ptr()).as_mut_ptr()).refcount += 1;
            Frc { ptr: self.ptr, phantom: PhantomData }
        }
    }
}

impl<T> Drop for Frc<T> {
    fn drop(&mut self) {
        unsafe {
            let inner = self.inner();
            if let Some(r2) = inner.refcount.checked_sub(1) {
                inner.refcount = r2;
            } else {
                let freelist = inner.list.inner.clone();
                (*self.ptr.as_ptr()).assume_init_drop();
                freelist.borrow_mut().free.push(self.ptr);
            }
        }
    }
}

impl<T> Drop for FreeListInner<T> {
    fn drop(&mut self) {
        unsafe {
            for x in self.free.iter() {
                mem::drop(Box::from_raw(x.as_ptr()));
            }
        }
    }
}

impl<T> Deref for Frc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe {
            &self.ptr.as_ref().assume_init_ref().value
        }
    }
}

#[test]
fn test_freelist() {
    let freelist = FreeList::new();
    let foo = freelist.alloc(vec![10; 1024]);
    let fooptr = foo.as_slice() as *const [i32];
    let bar = freelist.alloc(vec![20; 1024]);
    let barptr = bar.as_slice() as *const [i32];
    mem::drop(foo);
    let baz = freelist.alloc(vec![30; 1024]);
    let bazptr = baz.as_slice() as *const [i32];
    assert_eq!(fooptr, bazptr);
    assert_ne!(fooptr, barptr);
}

#[test]
fn test_freelist_hanging() {
    let freelist = FreeList::new();
    let foo = freelist.alloc(vec![vec![1]]);
    mem::drop(freelist);
    mem::drop(foo);
}