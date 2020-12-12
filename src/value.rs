use std::ops::{Add, Index, IndexMut, BitXor, Rem, Mul, Sub, BitAnd, Div, Shr, Shl, BitOr, Range, RangeFull, RangeTo, RangeFrom};
use std::mem::size_of;
use std::cmp::Ordering;
use crate::layout::{Layout, AggrLayout, Packing};
use std::fmt::{Debug, Formatter};
use std::{fmt, iter};
use llvm_ir::{TypeRef, IntPredicate};
use bitvec::vec::BitVec;
use bitvec::field::BitField;
use bitvec::order::Lsb0;
use crate::class::Class;
use bitvec::slice::BitSlice;
use smallvec::SmallVec;

//TODO
type Impl = Vec<u8>;

pub type ValueSlice = BitSlice<Lsb0, u8>;

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct Value {
    imp: Impl,
    bits: u64,
}

pub fn add_u64_i64(x: u64, y: i64) -> u64 {
    if y < 0 {
        x - ((-y) as u64)
    } else {
        x + (y as u64)
    }
}

impl Value {
    pub fn as_u128(&self) -> u128 {
        if self.imp.is_empty() {
            0
        } else {
            self[0..128.min(self.bits)].load_le()
        }
    }
    pub fn as_i128(&self) -> i128 {
        match self.bits() {
            8 => self.unwrap_i8() as i128,
            16 => self.unwrap_i16() as i128,
            32 => self.unwrap_i32() as i128,
            64 => self.unwrap_i64() as i128,
            128 => self.unwrap_i128() as i128,
            b => todo!("{:?}", b),
        }
    }
    pub fn as_u64(&self) -> u64 { self.as_u128() as u64 }
    pub fn as_i64(&self) -> i64 { self.as_i128() as i64 }
    pub fn bits(&self) -> u64 {
        self.bits
    }
    pub fn unwrap_int(&self, expected_bits: u64) -> u128 {
        assert_eq!(self.bits(), expected_bits);
        self.as_u128()
    }
    pub fn unwrap_bool(&self) -> bool { self.unwrap_int(1) != 0 }
    pub fn unwrap_u8(&self) -> u8 { self.unwrap_int(8) as u8 }
    pub fn unwrap_i8(&self) -> i8 { self.unwrap_int(8) as i8 }
    pub fn unwrap_u16(&self) -> u16 { self.unwrap_int(16) as u16 }
    pub fn unwrap_i16(&self) -> i16 { self.unwrap_int(16) as i16 }
    pub fn unwrap_u32(&self) -> u32 { self.unwrap_int(32) as u32 }
    pub fn unwrap_i32(&self) -> i32 { self.unwrap_int(32) as i32 }
    pub fn unwrap_u64(&self) -> u64 { self.unwrap_int(64) as u64 }
    pub fn unwrap_i64(&self) -> i64 { self.unwrap_int(64) as i64 }
    pub fn unwrap_u128(&self) -> u128 { self.unwrap_int(128) as u128 }
    pub fn unwrap_i128(&self) -> i128 { self.unwrap_int(128) as i128 }

    pub fn from_bits(slice: &ValueSlice) -> Self {
        let mut result = Self::zero(slice.len() as u64);
        result[..].copy_from_bitslice(slice);
        result
    }
    pub fn new(bits: u64, value: u128) -> Self {
        let mut result = Value::zero(bits);
        if bits > 0 {
            result[0..bits.min(128)].store(value);
        }
        result
    }
    pub fn zero(bits: u64) -> Self {
        let mut imp = Impl::new();
        imp.resize(((bits + 7) / 8) as usize, 0);
        Value { imp, bits }
    }
    pub fn as_bytes(&self) -> &[u8] {
        self.imp.as_slice()
    }
    pub fn from_bytes(bytes: &[u8], bits: u64) -> Self {
        assert_eq!(bytes.len() as u64, (bits + 7) / 8);
        let mut imp=Impl::with_capacity(bytes.len());
        imp.extend_from_slice(bytes);
        Value { imp, bits }
    }
    pub fn from_bytes_exact(bytes: &[u8]) -> Self {
        Self::from_bytes(bytes, (bytes.len() * 8) as u64)
    }
    pub fn aggregate(class: &Class, vs: impl Iterator<Item=Value>) -> Self {
        let layout = class.layout();
        let mut result = Value::zero(layout.bits());
        for (i, v) in vs.enumerate() {
            result.insert_bits(class.element(i as i64).bit_offset, &v);
        }
        result
    }
    pub fn extract(&self, class: &Class, index: i64) -> Value {
        let element = class.element(index);
        self.extract_bits(element.bit_offset, element.class.layout().bits())
    }
    pub fn extract_bits(&self, offset: i64, len: u64) -> Value {
        Value::from_bits(&self[offset as u64..offset as u64 + len as u64])
    }
    pub fn insert_bits(&mut self, offset: i64, value: &Value) {
        let left = &mut self[offset as u64..offset as u64 + value.bits()];
        let right = &value[..];
        left.copy_from_bitslice(right);
    }
    pub fn insert(&mut self, class: &Class, index: i64, value: &Value) {
        let element = class.element(index);
        assert_eq!(element.class.layout().bits(), value.bits());
        self.insert_bits(element.bit_offset, value);
    }
    pub fn ucast(&self, layout: Layout) -> Value {
        let mut result = Self::zero(layout.bits());
        let len = self.bits.min(layout.bits());
        result[..len].copy_from_bitslice(&self[..len]);
        result
    }
}

impl From<()> for Value {
    fn from(_: ()) -> Self {
        Value::new(0, 0)
    }
}

impl From<i8> for Value {
    fn from(value: i8) -> Self {
        Value::new(8, value as u128)
    }
}

impl From<i16> for Value {
    fn from(value: i16) -> Self {
        Value::new(16, value as u128)
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Value::new(32, value as u128)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::new(64, value as u128)
    }
}

impl From<i128> for Value {
    fn from(value: i128) -> Self {
        Value::new(128, value as u128)
    }
}

impl From<u8> for Value {
    fn from(value: u8) -> Self {
        Value::new(8, value as u128)
    }
}

impl From<u16> for Value {
    fn from(value: u16) -> Self {
        Value::new(16, value as u128)
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Value::new(32, value as u128)
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Value::new(64, value as u128)
    }
}

impl From<u128> for Value {
    fn from(value: u128) -> Self {
        Value::new(128, value as u128)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::new(1, value as u128)
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Value::new((size_of::<usize>() * 8) as u64, value as u128)
    }
}

impl Debug for Value {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.bits() == 0 {
            write!(f, "()")?;
        } else if self.bits() == 1 {
            write!(f, "{}", self.unwrap_bool())?;
        } else {
            assert_eq!(self.bits() % 8, 0);
            for x in self.imp.as_slice() {
                write!(f, "{:02X}", x)?;
            }
        }
        Ok(())
    }
}

impl Default for Value {
    fn default() -> Self {
        Value::from(())
    }
}

impl Index<RangeFull> for Value {
    type Output = ValueSlice;

    fn index(&self, index: RangeFull) -> &Self::Output {
        &BitSlice::from_slice(self.imp.as_slice()).unwrap()[..self.bits as usize]
    }
}

impl Index<u64> for Value {
    type Output = bool;
    fn index(&self, index: u64) -> &Self::Output {
        &self[..][index as usize]
    }
}

impl Index<Range<u64>> for Value {
    type Output = ValueSlice;
    fn index(&self, index: Range<u64>) -> &Self::Output {
        &self[..][index.start as usize..index.end as usize]
    }
}

impl Index<RangeTo<u64>> for Value {
    type Output = ValueSlice;
    fn index(&self, index: RangeTo<u64>) -> &Self::Output {
        &self[..][..index.end as usize]
    }
}

impl Index<RangeFrom<u64>> for Value {
    type Output = ValueSlice;
    fn index(&self, index: RangeFrom<u64>) -> &Self::Output {
        &self[..][index.start as usize..]
    }
}

impl IndexMut<RangeFull> for Value {
    fn index_mut(&mut self, index: RangeFull) -> &mut Self::Output {
        &mut BitSlice::from_slice_mut(self.imp.as_mut()).unwrap()[..self.bits as usize]
    }
}

impl IndexMut<Range<u64>> for Value {
    fn index_mut(&mut self, index: Range<u64>) -> &mut Self::Output {
        &mut self[..][index.start as usize..index.end as usize]
    }
}

impl IndexMut<RangeTo<u64>> for Value {
    fn index_mut(&mut self, index: RangeTo<u64>) -> &mut Self::Output {
        &mut self[..][..index.end as usize]
    }
}

impl IndexMut<RangeFrom<u64>> for Value {
    fn index_mut(&mut self, index: RangeFrom<u64>) -> &mut Self::Output {
        &mut self[..][index.start as usize..]
    }
}