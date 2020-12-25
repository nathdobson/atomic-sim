use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::fmt;
use std::lazy::OnceCell;
use std::ops::Deref;

#[derive(Debug, Copy, Clone)]
pub struct RecursiveError;

impl Display for RecursiveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Lazy value forced recursively.")
    }
}

impl Error for RecursiveError {}

pub struct Lazy<T, F: FnOnce() -> T = Box<dyn FnOnce() -> T>> {
    producer: Cell<Option<F>>,
    value: OnceCell<T>,
}

impl<T, F: FnOnce() -> T> Lazy<T, F> {
    fn new_unboxed(f: F) -> Self {
        Lazy {
            producer: Cell::new(Some(f)),
            value: OnceCell::new(),
        }
    }
    pub fn force(&self) -> Result<&T, RecursiveError> {
        if let Some(value) = self.value.get() {
            Ok(value)
        } else if let Some(producer) = self.producer.take() {
            assert!(self.value.set(producer()).is_ok());
            Ok(self.value.get().unwrap())
        } else {
            Err(RecursiveError)
        }
    }
}

impl<T> Lazy<T> {
    pub fn new(f: impl FnOnce() -> T + 'static) -> Self {
        Self::new_unboxed(Box::new(f))
    }
}

impl<T, F: FnOnce() -> T> From<T> for Lazy<T, F> {
    fn from(x: T) -> Self {
        Lazy {
            producer: Cell::new(None),
            value: OnceCell::from(x),
        }
    }
}

impl<T: Debug, F: FnOnce() -> T> Debug for Lazy<T, F> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.force())
    }
}

