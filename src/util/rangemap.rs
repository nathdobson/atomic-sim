use std::collections::BTreeMap;
use std::ops::{Range, RangeInclusive, Bound, RangeBounds, RangeFrom};
use std::collections::btree_map;
use std::cmp::Ordering;
use itertools::Itertools;
use std::{mem, fmt};
use std::fmt::{Debug, Formatter};
use std::iter::FromIterator;

#[derive(Clone)]
pub struct RangeMap<K: Ord + Copy, V: Clone> {
    map: BTreeMap<K, (K, V)>,
}

impl<K: Ord + Copy, V: Clone> RangeMap<K, V> {
    pub fn new() -> Self {
        RangeMap { map: BTreeMap::new() }
    }
    pub fn erase(&mut self, Range { start, end }: Range<K>) {
        while let Some((&v_end, &(v_start, _)))
        = self.map.range((Bound::Excluded(start), Bound::Unbounded)).next() {
            if v_start >= end {
                break;
            }
            let value = self.map.remove(&v_end).unwrap().1;
            if v_start < start {
                self.map.insert(start, (v_start, value.clone()));
            }
            if end < v_end {
                self.map.insert(v_end, (end, value.clone()));
            }
        }
    }
    pub fn insert(&mut self, Range { start, end }: Range<K>, value: V) {
        if start >= end {
            return;
        }
        self.erase(start..end);
        self.map.insert(end, (start, value));
    }
    pub fn range<R: RangeBounds<K>>(&self, r: R) -> impl Iterator<Item=(Range<K>, &V)> + '_ {
        let start = r.start_bound().cloned();
        let end = r.end_bound().cloned();
        let start_exclusive = match start {
            Bound::Included(x) => Bound::Excluded(x),
            Bound::Excluded(x) => todo!(),
            Bound::Unbounded => Bound::Unbounded,
        };
        self.map.range((start_exclusive, Bound::Unbounded))
            .map_while(move |(v_end, (v_start, value))| {
                let v_end = *v_end;
                let v_start = *v_start;
                let n_start = match start {
                    Bound::Included(start) => start.max(v_start),
                    Bound::Excluded(start) => todo!(),
                    Bound::Unbounded => v_start
                };
                let n_end = match end {
                    Bound::Included(end) => todo!(),
                    Bound::Excluded(end) => end.min(v_end),
                    Bound::Unbounded => v_end,
                };
                if n_start >= n_end {
                    None
                } else {
                    Some((n_start..n_end, value))
                }
            })
    }
}

impl<K: Ord + Copy + Debug, V: Clone + Debug> Debug for RangeMap<K, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.range(..)).finish()
    }
}

impl<K: Ord + Copy, V: Clone> FromIterator<(Range<K>, V)> for RangeMap<K, V> {
    fn from_iter<T: IntoIterator<Item=(Range<K>, V)>>(iter: T) -> Self {
        let mut result = RangeMap::new();
        for (r, v) in iter {
            result.insert(r, v);
        }
        result
    }
}

#[cfg(test)]
mod test {
    use std::collections::{BTreeMap, HashMap};
    use std::ops::{RangeInclusive, Range};
    use crate::rangemap::RangeMap;
    use itertools::Itertools;
    use rand_xorshift::XorShiftRng;
    use rand::{SeedableRng, Rng};

    #[test]
    fn test() {
        let mut table = RangeMap::new();
        fn test_range(table: &RangeMap<u64, u8>, range: Range<u64>, exp: Vec<(Range<u64>, u8)>) {
            assert_eq!(table.range(range).map(|(r, v)| (r, *v)).collect::<Vec<_>>(), exp);
        }
        test_range(&table, 0..0, vec![]);
        test_range(&table, u64::MAX..u64::MAX, vec![]);
        table.insert(10..20, 1);
        test_range(&table, 0..u64::MAX, vec![(10..20, 1)]);
        test_range(&table, 10..20, vec![(10..20, 1)]);
        test_range(&table, 10..30, vec![(10..20, 1)]);
        test_range(&table, 10..18, vec![(10..18, 1)]);
        test_range(&table, 12..20, vec![(12..20, 1)]);
        test_range(&table, 12..18, vec![(12..18, 1)]);
        table.insert(9..12, 2);
        test_range(&table, 0..u64::MAX, vec![(9..12, 2), (12..20, 1)]);
        table.insert(19..22, 3);
        test_range(&table, 0..u64::MAX, vec![(9..12, 2), (12..19, 1), (19..22, 3)]);
        table.insert(15..16, 4);
        test_range(&table, 0..u64::MAX, vec![(9..12, 2), (12..15, 1), (15..16, 4), (16..19, 1), (19..22, 3)]);
    }

    #[test]
    fn random_test() {
        for i in 0..1000 {
            println!("seed={:?}", i);
            let mut random = XorShiftRng::seed_from_u64(i);
            let mut table = RangeMap::new();
            let mut expected = BTreeMap::new();
            for value in 0..10u64 {
                {
                    let start = random.gen_range(0, 10);
                    let end = random.gen_range(start, 11);
                    let ranges: Vec<(Range<u64>, &u64)> = table.range(start..end).collect();
                    println!("range({:?})={:?}", start..end, ranges);
                    for (r, v) in ranges.iter() {
                        assert!(r.start >= start);
                        assert!(r.end <= end);
                    }
                    for ((r1, v1), (r2, v2)) in ranges.iter().tuple_windows() {
                        assert!(r1.end != r2.start || v1 != v2);
                    }
                    let mut actual = BTreeMap::new();
                    for (r, v) in ranges.iter() {
                        for k in r.clone() {
                            actual.insert(k, **v);
                        }
                    }
                    for k in start..end {
                        assert_eq!(actual.get(&k), expected.get(&k));
                    }
                }
                {
                    let start = random.gen_range(0, 10);
                    let end = random.gen_range(start, 11);
                    println!("insert({:?},{:?})", start..end, value);
                    println!("table={:?}", table);
                    table.insert(start..end, value);
                    for k in start..end {
                        expected.insert(k, value);
                    }
                    println!("expected={:?}", expected);
                }
            }
        }
    }
}