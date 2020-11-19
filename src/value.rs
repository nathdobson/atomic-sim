use std::ops::{Add, Index, IndexMut, BitXor, Rem, Mul, Sub, BitAnd, Div, Shr, Shl, BitOr};
use std::mem::size_of;
use std::cmp::Ordering;
use crate::layout::Layout;

#[non_exhaustive]
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum Value {
    Int { bits: u64, value: u128 },
    Aggregate { children: Vec<Value>, is_packed: bool },
}

pub fn add_u64_i64(x: u64, y: i64) -> u64 {
    if y < 0 {
        x - ((-y) as u64)
    }else{
        x + (y as u64)
    }
}

impl Value {
    // pub fn bits(&self) -> u64 {
    //     match self{
    //         Value::Int {bits, .. } => bits
    //         Value::Aggregate(xs) =>
    //     }
    // }
    //
    pub fn same_type(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int { bits: b1, .. }, Value::Int { bits: b2, .. }) => b1 == b2,
            (Value::Aggregate { children: c1, is_packed: p1 },
                Value::Aggregate { children: c2, is_packed: p2 }) =>
                c1.iter().zip(c2.iter()).all(|(x, y)| x.same_type(y)) && p1 == p2,
            _ => false
        }
    }
    pub fn as_u128(&self) -> u128 {
        match self {
            Value::Int { bits, value } => *value,
            Value::Aggregate { .. } => panic!()
        }
    }
    pub fn as_i128(&self) -> i128 {
        match self.int_bits() {
            8 => self.unwrap_i8() as i128,
            16 => self.unwrap_i16() as i128,
            32 => self.unwrap_i32() as i128,
            64 => self.unwrap_i64() as i128,
            b => todo!("{:?}", b),
        }
    }
    pub fn as_u64(&self) -> u64 { self.as_u128() as u64 }
    pub fn as_i64(&self) -> i64 { self.as_i128() as i64 }
    pub fn int_bits(&self) -> u64 {
        match self {
            Value::Int { bits, .. } => *bits,
            _ => panic!("Not an int: {:?}", self),
        }
    }
    pub fn unwrap_int(&self, expected_bits: u64) -> u128 {
        match self {
            Value::Int { bits, value } => {
                assert_eq!(expected_bits, *bits);
                *value
            }
            Value::Aggregate { .. } => panic!(),
        }
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

    pub fn new(bits: u64, value: u128) -> Self {
        Value::Int { bits, value }
    }
    // pub fn with_bytes(bits: u64, bytes: [u8; 16]) -> Self {
    //     Value::Int { bits, value: u128::from_le_bytes(bytes) }
    // }
    // pub fn layout(&self) -> Layout {
    //     match self {
    //         Value::Int { bits, .. } => Layout::of_int(*bits),
    //         Value::Aggregate(vs) => vs.iter().map(|v| v.layout()).collect::<Layout>().pad_to_align(),
    //     }
    // }
    pub fn sext(&self, to: u64) -> Value {
        match self {
            Value::Int { bits, value } => {
                match (bits, to) {
                    (8, 16) => Value::from(self.unwrap_i8() as i16),
                    (8, 32) => Value::from(self.unwrap_i8() as i32),
                    (8, 64) => Value::from(self.unwrap_i8() as i64),
                    (16, 32) => Value::from(self.unwrap_i16() as i32),
                    (32, 64) => Value::from(self.unwrap_i32() as i64),
                    _ => todo!("{:?} -> {:?}", bits, to),
                }
            }
            Value::Aggregate { .. } => panic!()
        }
    }
    pub fn aggregate(vs: impl Iterator<Item=Value>, is_packed: bool) -> Self {
        Value::Aggregate { children: vs.collect(), is_packed }
    }
    pub fn truncate(&self, bits: u64) -> Self {
        let mask = ((1u128 << bits) - 1);
        let result = Value::Int { bits, value: self.as_u128() & mask };
        result
    }
    pub fn sdiv(&self, rhs: &Self) -> Self {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_i64() / y.unwrap_i64()),
            (8, 8) => Value::from(x.unwrap_i8() / y.unwrap_i8()),
            bits => todo!("{:?}", bits),
        }
    }
    pub fn srem(&self, rhs: &Self) -> Self {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_i64() % y.unwrap_i64()),
            (8, 8) => Value::from(x.unwrap_i8() % y.unwrap_i8()),
            bits => todo!("{:?}", bits),
        }
    }
}

impl Index<u64> for Value {
    type Output = Value;

    fn index(&self, index: u64) -> &Self::Output {
        match self {
            Value::Int { .. } => panic!("Indexing an int"),
            Value::Aggregate { children, is_packed } => &children[index as usize],
        }
    }
}

impl IndexMut<u64> for Value {
    fn index_mut(&mut self, index: u64) -> &mut Self::Output {
        match self {
            Value::Int { .. } => panic!("Indexing an int"),
            Value::Aggregate { children, is_packed } => &mut children[index as usize],
        }
    }
}

impl From<()> for Value {
    fn from(_: ()) -> Self {
        Value::Int { bits: 0, value: 0u128 }
    }
}

impl From<i8> for Value {
    fn from(value: i8) -> Self { Value::Int { bits: 8, value: value as u8 as u128 } }
}

impl From<i16> for Value {
    fn from(value: i16) -> Self { Value::Int { bits: 16, value: value as u16 as u128 } }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self { Value::Int { bits: 32, value: value as u32 as u128 } }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self { Value::Int { bits: 64, value: value as u64 as u128 } }
}

impl From<i128> for Value {
    fn from(value: i128) -> Self { Value::Int { bits: 128, value: value as u128 } }
}

impl From<u8> for Value {
    fn from(value: u8) -> Self {
        Value::Int { bits: 8, value: value as u128 }
    }
}

impl From<u16> for Value {
    fn from(value: u16) -> Self {
        Value::Int { bits: 16, value: value as u128 }
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Value::Int { bits: 32, value: value as u128 }
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Value::Int { bits: 64, value: value as u128 }
    }
}

impl From<u128> for Value {
    fn from(value: u128) -> Self {
        Value::Int { bits: 128, value: value }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Int { bits: 1, value: if value { 1 } else { 0 } }
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Value::Int { bits: (size_of::<usize>() * 8) as u64, value: value as u128 }
    }
}

impl Add for &Value {
    type Output = Value;
    fn add(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.as_u64() + y.as_u64()),
            bits => todo!("{:?}", bits)
        }
    }
}

impl BitXor for &Value {
    type Output = Value;
    fn bitxor(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (1, 1) => Value::from(x.unwrap_bool() ^ y.unwrap_bool()),
            (64, 64) => Value::from(x.unwrap_u64() ^ y.unwrap_u64()),
            bits => todo!("{:?}", bits)
        }
    }
}

impl Rem for &Value {
    type Output = Value;

    fn rem(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_u64() % y.unwrap_u64()),
            bits => todo!("{:?}", bits),
        }
    }
}

impl Mul for &Value {
    type Output = Value;

    fn mul(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_u64() * y.unwrap_u64()),
            bits => todo!("{:?}", bits),
        }
    }
}


impl Sub for &Value {
    type Output = Value;

    fn sub(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_u64().wrapping_sub(y.unwrap_u64())),
            (8, 8) => Value::from(x.unwrap_u8().wrapping_sub( y.unwrap_u8())),
            bits => todo!("{:?}", bits),
        }
    }
}

impl BitAnd for &Value {
    type Output = Value;

    fn bitand(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (8, 8) => Value::from(x.unwrap_u8() & y.unwrap_u8()),
            (32, 32) => Value::from(x.unwrap_u32() & y.unwrap_u32()),
            (64, 64) => Value::from(x.unwrap_u64() & y.unwrap_u64()),
            bits => todo!("{:?}", bits),
        }
    }
}

impl Div for &Value {
    type Output = Value;

    fn div(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_u64() / y.unwrap_u64()),
            (8, 8) => Value::from(x.unwrap_u8() / y.unwrap_u8()),
            bits => todo!("{:?}", bits),
        }
    }
}

impl Shr for &Value {
    type Output = Value;

    fn shr(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_u64() >> y.unwrap_u64()),
            (8, 8) => Value::from(x.unwrap_u8() >> y.unwrap_u8()),
            bits => todo!("{:?}", bits),
        }
    }
}

impl BitOr for &Value {
    type Output = Value;

    fn bitor(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_u64() | y.unwrap_u64()),
            (8, 8) => Value::from(x.unwrap_u8() | y.unwrap_u8()),
            bits => todo!("{:?}", bits),
        }
    }
}

impl Shl for &Value {
    type Output = Value;

    fn shl(self, rhs: Self) -> Self::Output {
        let (x, y) = (self, rhs);
        match (x.int_bits(), y.int_bits()) {
            (64, 64) => Value::from(x.unwrap_u64() << y.unwrap_u64()),
            (8, 8) => Value::from(x.unwrap_u8() << y.unwrap_u8()),
            bits => todo!("{:?}", bits),
        }
    }
}