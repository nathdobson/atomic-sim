use std::iter::FromIterator;

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct Layout { size: u64, align: u64 }

pub fn align_to(ptr: u64, align: u64) -> u64 {
    (ptr.wrapping_sub(1) | (align - 1)).wrapping_add(1)
}

impl Layout {
    pub fn from_size_align(size: u64, align: u64) -> Self {
        assert!(align.is_power_of_two());
        Layout { size, align }
    }
    pub fn extend(&self, packed: bool, other: Self) -> Self {
        if packed {
            Layout {
                size: self.size + other.size,
                align: 1,
            }
        } else {
            Layout {
                size: align_to(self.size, other.align) + other.size,
                align: self.align.max(other.align),
            }
        }
    }
    pub fn offset(&self, packed: bool, other: Layout) -> u64 {
        if packed {
            self.size
        } else {
            align_to(self.size, other.align)
        }
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn align(&self) -> u64 { self.align }
    pub fn of_int(bits: u64) -> Layout {
        let size = ((bits + 7) / 8) as u64;
        Self::from_size_align(size, size.max(1))
    }
    pub fn pad_to_align(&self) -> Self {
        Layout::from_size_align(align_to(self.size, self.align), self.align)
    }
    pub fn aggregate<T: IntoIterator<Item=Layout>>(packed: bool, iter: T) -> Self {
        let mut result = Layout::from_size_align(0, 1);
        for f in iter {
            result = result.extend(packed, f);
        }
        result
    }
}
