/*
 * Released under the terms of the Apache 2.0 license with LLVM
 * exception. See `LICENSE` for details.
 */

//! Sets of integer indices for liveness analysis and other purposes.

use crate::bitvec::*;
use smallvec::{smallvec, SmallVec};
use std::convert::TryFrom;

/// If any index is >= this threshold, we switch to sparse mode.
const SPARSE_THRESHOLD: usize = 512;
/// If we have more than this many elements, we sort before probing
/// (otherwise we do a linear search).
const SORT_THRESHOLD: usize = 16;

type ListSmallVec = SmallVec<[u32; 4]>;

/// An IntSet is a set of integers that uses a hybrid scheme to be
/// efficient for both dense and sparse data. Based on the maximal
/// index, it switches modes between a dense bitvector and an unsorted
/// or sorted list.
#[derive(Clone, Debug)]
pub enum IntSet {
    /// Empty.
    Empty,
    /// Simple bitvector: bit set for every present integer.
    Small(BitVec),
    /// Unsorted list of integers, possibly with duplicates.
    Unsorted(ListSmallVec),
    /// Sorted list of integers, with all duplicates removed.
    Sorted(ListSmallVec),
}

impl std::default::Default for IntSet {
    fn default() -> Self {
        Self::Empty
    }
}

fn remove_dups(list: &mut ListSmallVec) {
    let mut out_idx = 0;
    let mut last = None;
    for i in 0..list.len() {
        if Some(list[i]) != last {
            if out_idx < i {
                list[out_idx] = list[i];
            }
            out_idx += 1;
        }
        last = Some(list[i]);
    }
    list.truncate(out_idx);
}

impl IntSet {
    /// Create a new IntSet.
    pub fn new() -> Self {
        Self::Empty
    }

    /// Clear the set.
    pub fn clear(&mut self) {
        *self = Self::Empty;
    }

    /// Is the set empty?
    pub fn is_empty(&self) -> bool {
        match self {
            &Self::Empty => true,
            &Self::Small(ref bv) => bv.iter().next().is_none(),
            &Self::Unsorted(ref list) | &Self::Sorted(ref list) => list.is_empty(),
        }
    }

    /// Add a value.
    pub fn add(&mut self, val: usize) {
        let u32_val = u32::try_from(val).expect("out of range");
        let new_self = match std::mem::replace(self, Self::Empty) {
            Self::Empty => {
                if val >= SPARSE_THRESHOLD {
                    let list: ListSmallVec = smallvec![u32_val];
                    Self::Sorted(list)
                } else {
                    let mut bv = BitVec::new();
                    bv.set(val as usize, true);
                    Self::Small(bv)
                }
            }
            Self::Small(mut bv) => {
                if val >= SPARSE_THRESHOLD {
                    let mut list: ListSmallVec = bv.iter().map(|val| val as u32).collect();
                    list.push(u32_val);
                    Self::Unsorted(list)
                } else {
                    bv.set(val as usize, true);
                    Self::Small(bv)
                }
            }
            Self::Unsorted(mut list) => {
                list.push(u32_val);
                Self::Unsorted(list)
            }
            Self::Sorted(mut list) => {
                list.push(u32_val);
                Self::Unsorted(list)
            }
        };
        *self = new_self;
    }

    /// Remove a value.
    pub fn remove(&mut self, val: usize) {
        let u32_val = u32::try_from(val).expect("out of range");
        let new_self = match std::mem::replace(self, Self::Empty) {
            Self::Empty => Self::Empty,
            Self::Small(mut bv) => {
                bv.set(val, false);
                Self::Small(bv)
            }
            Self::Unsorted(mut list) => {
                list.retain(|elem| *elem != u32_val);
                Self::Unsorted(list)
            }
            Self::Sorted(mut list) => {
                if let Ok(idx) = list.as_slice().binary_search(&u32_val) {
                    list.remove(idx);
                }
                Self::Sorted(list)
            }
        };
        *self = new_self;
    }

    /// Probe for a value.
    pub fn contains(&mut self, val: usize) -> bool {
        match &*self {
            &Self::Unsorted(ref l) if l.len() >= SORT_THRESHOLD => {
                self.sort();
            }
            _ => {}
        }

        let u32_val = u32::try_from(val).expect("out of range");
        match &*self {
            &Self::Empty => false,
            &Self::Small(ref bv) => bv.get(val),
            &Self::Unsorted(ref list) => list.iter().any(|elem| *elem == u32_val),
            &Self::Sorted(ref list) => list.as_slice().binary_search(&u32_val).is_ok(),
        }
    }

    /// Merge in another set (mutate this set to the union of the
    /// two).  Returns `true` if any value was actually added.
    ///
    /// `other` is given as a mut borrow to allow it to be lazily
    /// sorted if previously unsorted, but semantically its contents
    /// are not changed.
    pub fn merge(&mut self, other: &mut Self) -> bool {
        // Ensure both sides are sorted.
        self.sort();
        other.sort();

        let (new_self, changed) = match (std::mem::replace(self, Self::Empty), &*other) {
            (Self::Unsorted(..), _) => unreachable!(),
            (_, &Self::Unsorted(..)) => unreachable!(),
            (x, &Self::Empty) => (x, false),
            (Self::Empty, other) => (other.clone(), !other.is_empty()),
            (Self::Small(mut bv), &Self::Small(ref other)) => {
                let changed = bv.or(other);
                (Self::Small(bv), changed)
            }
            (Self::Small(bv), &Self::Sorted(ref list)) => {
                let mut list = list.clone();
                let changed = list.iter().any(|elem| !bv.get(*elem as usize));
                for idx in bv.iter() {
                    list.push(idx as u32);
                }
                (Self::Unsorted(list), changed)
            }
            (Self::Sorted(mut list), &Self::Small(ref bv)) => {
                let mut changed = false;
                let orig_len = list.len();
                for idx in bv.iter() {
                    let idx = idx as u32;
                    if !list.as_slice()[0..orig_len].binary_search(&idx).is_ok() {
                        changed = true;
                        list.push(idx);
                    }
                }
                (Self::Unsorted(list), changed)
            }
            (Self::Sorted(l1), &Self::Sorted(ref l2)) => {
                let mut changed = false;
                let mut merged = smallvec![];
                let mut i = 0;
                let mut j = 0;
                while i < l1.len() || j < l2.len() {
                    if i < l1.len() && j < l2.len() {
                        if l1[i] == l2[j] {
                            merged.push(l1[i]);
                            i += 1;
                            j += 1;
                        } else if l1[i] < l2[j] {
                            merged.push(l1[i]);
                            i += 1;
                        } else if l2[j] < l1[i] {
                            merged.push(l2[j]);
                            j += 1;
                            changed = true;
                        }
                    } else if i < l1.len() {
                        merged.push(l1[i]);
                        i += 1;
                    } else if j < l2.len() {
                        merged.push(l2[j]);
                        j += 1;
                        changed = true;
                    }
                }
                (Self::Sorted(merged), changed)
            }
        };
        *self = new_self;
        changed
    }

    /// Sort items if unsorted.
    pub fn sort(&mut self) {
        let new_self = match std::mem::replace(self, Self::Empty) {
            Self::Unsorted(mut list) => {
                list.sort();
                remove_dups(&mut list);
                Self::Sorted(list)
            }
            x => x,
        };
        *self = new_self;
    }

    /// Get an iterator over items.
    pub fn iter<'a>(&'a self) -> SetIter<'a> {
        match self {
            &Self::Empty => SetIter::Empty,
            &Self::Small(ref bv) => SetIter::BitVec(bv.iter()),
            &Self::Unsorted(ref list) | &Self::Sorted(ref list) => SetIter::Slice(list.as_slice()),
        }
    }
}

pub enum SetIter<'a> {
    Empty,
    Slice(&'a [u32]),
    BitVec(SetBitsIter<'a>),
}

impl<'a> std::iter::Iterator for SetIter<'a> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        let (ret, new_self) = match std::mem::replace(self, SetIter::Empty) {
            Self::Empty => (None, Self::Empty),
            Self::Slice(slice) if slice.len() > 1 => {
                (Some(slice[0] as usize), Self::Slice(&slice[1..]))
            }
            Self::Slice(slice) if slice.len() == 1 => (Some(slice[0] as usize), Self::Empty),
            Self::Slice(slice) => {
                assert!(slice.is_empty());
                (None, Self::Empty)
            }
            Self::BitVec(mut iter) => {
                let next = iter.next();
                (next, Self::BitVec(iter))
            }
        };
        *self = new_self;
        ret
    }
}
