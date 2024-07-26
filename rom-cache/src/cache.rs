//! Cache data structure

use crate::error::CacheResult;

use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::any::Any;
use std::mem::{size_of, MaybeUninit};
use std::ptr;

/// A cache data structure.
#[derive(Debug)]
#[repr(transparent)]
pub struct Cache<const G: usize, const L: usize> {
    groups: [CacheGroup<L>; G],
}

impl<const G: usize, const L: usize> Default for Cache<G, L> {
    fn default() -> Self {
        let groups = unsafe { MaybeUninit::<[CacheGroup<L>; G]>::zeroed().assume_init() };
        Self { groups }
    }
}

#[derive(Debug)]
#[repr(transparent)]
struct CacheGroup<const L: usize> {
    lines: [CacheLine; L],
}

#[derive(Debug)]
#[repr(transparent)]
struct CacheLine {
    value: usize,
}

trait Cacheable: Any {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache() {
        let cache: Cache<8, 2> = Default::default();
    }
}
