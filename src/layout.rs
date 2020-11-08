pub struct Layout { size: u64, align: u64 }

pub fn align_to(ptr: u64, align: u64) -> u64 {
    (ptr.wrapping_sub(1) | (align - 1)).wrapping_add(1)
}

impl Layout {
    pub fn from_size_align(size: u64, align: u64) -> Self {
        assert!(align.is_power_of_two());
        Layout { size, align }
    }
    pub fn extend(&self, other: Self) -> Self {
        Layout {
            size: align_to(self.size, other.align) + other.size,
            align: self.align.max(other.align),
        }
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn align(&self) -> u64 { self.align }
}

