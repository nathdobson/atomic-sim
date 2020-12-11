use std::ops::{Deref, DerefMut};
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};

#[derive(Clone, Debug)]
pub struct ByAddress<T: Address>(pub T);

impl<T: Address> PartialEq for ByAddress<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_ptr() == other.0.as_ptr()
    }
}

impl<T: Address> PartialOrd for ByAddress<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.as_ptr().partial_cmp(&other.0.as_ptr())
    }
}

impl<T: Address> Ord for ByAddress<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.as_ptr().cmp(&other.0.as_ptr())
    }
}

impl<T: Address> Hash for ByAddress<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_ptr().hash(state)
    }
}

impl<T: Address> Eq for ByAddress<T> {}

pub trait Address {
    type Target;
    fn as_ptr(&self) -> *const Self::Target;
}

impl<T: Address + Deref> Deref for ByAddress<T> {
    type Target = <T as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: Address + DerefMut> DerefMut for ByAddress<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl<T> Address for Rc<T> {
    type Target = T;

    fn as_ptr(&self) -> *const T {
        Rc::as_ptr(self)
    }
}

impl<T> Address for Weak<T> {
    type Target = T;
    fn as_ptr(&self) -> *const T {
        self.as_ptr()
    }
}

impl<T> ByAddress<Weak<T>> {
    pub fn upgrade(&self) -> Option<ByAddress<Rc<T>>> {
        Some(ByAddress(self.0.upgrade()?))
    }
}

impl<T> ByAddress<Rc<T>> {
    pub fn downgrade(this: &Self) -> ByAddress<Weak<T>> {
        ByAddress(Rc::downgrade(&this.0))
    }
}