use std::cmp::Ordering;
use std::fmt;
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::rc::{Rc, Weak};

use smallvec::alloc::fmt::Formatter;

#[derive(Clone, Debug)]
pub struct ByAddress<T: Address + ?Sized>(pub T);

impl<T: Address + ?Sized> PartialEq for ByAddress<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_ptr() == other.0.as_ptr()
    }
}

impl<T: Address + ?Sized> PartialOrd for ByAddress<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.as_ptr().partial_cmp(&other.0.as_ptr())
    }
}

impl<T: Address + ?Sized> Ord for ByAddress<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.as_ptr().cmp(&other.0.as_ptr())
    }
}

impl<T: Address + ?Sized> Hash for ByAddress<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_ptr().hash(state)
    }
}

impl<T: Address + ?Sized> Eq for ByAddress<T> {}

pub trait Address {
    type Target: ?Sized;
    fn as_ptr(&self) -> *const Self::Target;
}

impl<T: Address + Deref + ?Sized> Deref for ByAddress<T> {
    type Target = <T as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: Address + DerefMut + ?Sized> DerefMut for ByAddress<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl<T: ?Sized> Address for Rc<T> {
    type Target = T;

    fn as_ptr(&self) -> *const T {
        Rc::as_ptr(self)
    }
}

impl<T: ?Sized> Address for Weak<T> {
    type Target = T;
    fn as_ptr(&self) -> *const T {
        self.as_ptr()
    }
}

impl<T: ?Sized> ByAddress<Weak<T>> {
    pub fn upgrade(&self) -> Option<ByAddress<Rc<T>>> {
        Some(ByAddress(self.0.upgrade()?))
    }
}

impl<T: ?Sized> ByAddress<Rc<T>> {
    pub fn downgrade(this: &Self) -> ByAddress<Weak<T>> {
        ByAddress(Rc::downgrade(&this.0))
    }
}

impl<T: ?Sized> Address for &T {
    type Target = T;

    fn as_ptr(&self) -> *const T {
        (*self) as *const T
    }
}

impl<T: Display + ?Sized + Address> Display for ByAddress<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}