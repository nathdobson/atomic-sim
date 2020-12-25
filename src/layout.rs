use std::fmt::{Debug, Formatter};
use std::fmt;
use std::iter::FromIterator;

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct Layout { bits: u64, bit_align: u64 }

pub fn align_to(ptr: u64, align: u64) -> u64 {
    (ptr.wrapping_sub(1) | (align - 1)).wrapping_add(1)
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct AggrLayout {
    bit_offsets: Vec<u64>,
    layout: Layout,
}

#[derive(Copy, Clone, Eq, Ord, PartialEq, PartialOrd, Debug, Hash)]
pub enum Packing {
    None,
    Bit,
    Byte,
}

impl Layout {
    pub fn from_bytes(bytes: u64, byte_align: u64) -> Self {
        Self { bits: bytes * 8, bit_align: byte_align * 8 }
    }
    pub fn from_bits(bits: u64, bit_align: u64) -> Self {
        assert!(bit_align >= 8);
        assert!(bit_align.is_power_of_two());
        Self { bits, bit_align }
    }
    pub fn extend(&self, packing: Packing, other: Self) -> (Self, i64) {
        match packing {
            Packing::None => {
                let offset = align_to(self.bits, other.bit_align);
                (Self {
                    bits: offset + other.bits,
                    bit_align: self.bit_align.max(other.bit_align),
                }, offset as i64)
            }
            Packing::Bit => {
                let offset = self.bits;
                (Self { bits: offset + other.bits, bit_align: 8 }, offset as i64)
            }
            Packing::Byte => {
                let offset = align_to(self.bits, 8);
                (Self { bits: offset + other.bits, bit_align: 8 }, offset as i64)
            }
        }
    }
    pub fn bytes(&self) -> u64 {
        (self.bits + 7) / 8
    }
    pub fn bits(&self) -> u64 {
        self.bits
    }
    pub fn bit_align(&self) -> u64 { self.bit_align }
    pub fn byte_align(&self) -> u64 { (self.bit_align + 7) / 8 }
    pub fn of_int(bits: u64) -> Layout {
        let align = bits.max(8).next_power_of_two();
        Self::from_bits(bits, align)
    }
    pub fn of_func() -> Layout { Self::from_bytes(256, 1) }
    pub fn pad_to_align(&self) -> Self {
        Layout::from_bits(align_to(self.bits, self.bit_align), self.bit_align)
    }
    pub fn align_to_bits(&self, align_bits: u64) -> Self {
        Layout::from_bits(self.bits, self.bit_align.max(align_bits))
    }
    pub fn repeat(&self, packing: Packing, n: u64) -> Self {
        match packing {
            Packing::None => {
                assert_eq!(self.bit_align % 8, 0);
                Layout::from_bits(align_to(self.bits, self.bit_align) * n, self.bit_align)
            }
            Packing::Bit => {
                Layout::from_bits(self.bits * n, 8)
            }
            Packing::Byte => {
                assert_eq!(self.bit_align % 8, 0);
                Layout::from_bits(align_to(self.bits, 8) * n, 8)
            }
        }
    }
}

impl Debug for Layout {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "b{}/{}", self.bits, self.bit_align)
    }
}

impl From<bool> for Packing {
    fn from(is_packed: bool) -> Self {
        if is_packed {
            Packing::Byte
        } else {
            Packing::None
        }
    }
}