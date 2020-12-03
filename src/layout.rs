use std::iter::FromIterator;
use std::fmt::{Debug, Formatter};
use std::fmt;

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
    fn extend(&self, packing: Packing, other: Self) -> (Self, u64) {
        match packing {
            Packing::None => {
                let offset = align_to(self.bits, other.bit_align);
                (Self {
                    bits: offset + other.bits,
                    bit_align: self.bit_align.max(other.bit_align),
                }, offset)
            }
            Packing::Bit => {
                let offset = self.bits;
                (Self { bits: offset + other.bits, bit_align: 8 }, offset)
            }
            Packing::Byte => {
                let offset = align_to(self.bits, 8);
                (Self { bits: offset + other.bits, bit_align: 8 }, offset)
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
        Self::from_bits(bits, bits.max(8))
    }
    pub fn pad_to_align(&self) -> Self {
        Layout::from_bits(align_to(self.bits, self.bit_align), self.bit_align)
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

impl AggrLayout {
    pub fn new(packing: Packing, iter: impl Iterator<Item=Layout>) -> Self {
        let mut result = Layout::from_bits(0, 8);
        let mut bit_offsets = vec![];
        for layout in iter {
            let (new, off) = result.extend(packing, layout);
            result = new;
            bit_offsets.push(off);
        }
        Self {
            bit_offsets,
            layout: result.pad_to_align(),
        }
    }
    pub fn layout(&self) -> Layout {
        self.layout
    }
    pub fn bit_offset(&self, index: usize) -> u64 {
        self.bit_offsets[index]
    }
}

impl Debug for Layout {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "b{}%{}", self.bits, self.bit_align)
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