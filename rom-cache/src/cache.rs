//! Cache data structure

use crate::error::CacheResult;

use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::any::{Any, TypeId};
use std::cell::UnsafeCell;
use std::mem::{size_of, transmute, MaybeUninit};
use std::ptr;

/// A cache data structure.
/// - G: the number of cache groups
/// - L: the number of cache lines in each group
/// - B: the size of each cache line block in bytes
#[derive(Debug)]
pub struct Cache<const G: usize, const L: usize, const B: usize> {
    groups: [CacheGroup<L, B>; G],
}

impl<const G: usize, const L: usize, const B: usize> Default for Cache<G, L, B> {
    fn default() -> Self {
        #[cfg(debug_assertions)]
        {
            assert!(G > 0, "Invalid number of cache groups {}.", G);
            assert!(L > 0, "Invalid number of cache lines {}.", L);
        }
        unsafe { MaybeUninit::zeroed().assume_init() }
    }
}

impl<const G: usize, const L: usize, const B: usize> Cache<G, L, B> {
    /// load a Cacheable into memory
    pub fn load<T: Cacheable>(&mut self) {
        T::load_to(self);
    }

    /// Get a Cacheable from the cache.
    pub fn get<T: Cacheable>(&self) -> CacheResult<&T> {
        todo!()
    }
}

#[derive(Debug)]
struct CacheGroup<const L: usize, const B: usize> {
    lines: [CacheLine<B>; L],
}

impl<const L: usize, const B: usize> CacheGroup<L, B> {
    /// load Cacheable as CacheLine into the cache.
    fn load<T>(&mut self, value: T, type_id: usize) {
        let mut slug = (None, None, None);
        for (i, line) in self.lines.iter().enumerate() {
            if line.type_id == type_id {
                slug.0 = Some(i); // hit
                continue;
            } else if slug.1.is_none() && line.ptr.is_null() {
                slug.1 = Some(i); // empty
                continue;
            } else if slug.1.is_none() && line.lru as usize == L - 1 {
                slug.2 = Some(i); // expired
            }
        }
        match slug {
            // hit
            (Some(i), _, _) => {
                let lru = self.lines[i].lru;
                self.lines
                    .iter_mut()
                    .filter(|l| l.lru < lru)
                    .for_each(|l| l.lru += 1);
                self.lines[i].lru = 0;
                unsafe {
                    (self.lines[i].ptr as *mut T).write(value);
                }
            }
            // empty
            (_, Some(i), None) => {
                self.lines.iter_mut().for_each(|l| l.lru += 1);
                self.lines[i].lru = 0;
                unsafe {
                    self.lines[i].ptr = alloc(Layout::from_size_align_unchecked(B, 8));
                    (self.lines[i].ptr as *mut T).write(value);
                }
                self.lines[i].type_id = type_id;
            }
            // expired
            (_, _, Some(i)) => {
                self.lines.iter_mut().for_each(|l| l.lru += 1);
                self.lines[i].lru = 0;
                unsafe {
                    let layout = Layout::from_size_align_unchecked(B, 8);
                    dealloc(self.lines[i].ptr, layout);
                    self.lines[i].ptr = alloc(layout);
                    (self.lines[i].ptr as *mut T).write(value);
                }
                self.lines[i].type_id = type_id;
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
struct CacheLine<const B: usize> {
    ptr: *mut u8,
    flag: u8,
    lru: u8,
    type_id: usize,
}

impl<const B: usize> Drop for CacheLine<B> {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { dealloc(self.ptr, Layout::from_size_align_unchecked(B, 8)) };
        }
    }
}

/// A type that can be cached.
pub trait Cacheable: Any + Default + Sized {
    /// Load Cachable from the storage to cache.
    fn load() -> CacheResult<Self>;
    /// Write Cachable back to storage.
    fn store(&self) -> CacheResult<()>;
    /// Load Cachable from the storage to cache, or return the default value.
    fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }
    /// load Cachable into the cache.
    fn load_to<const G: usize, const L: usize, const B: usize>(cache: &mut Cache<G, L, B>) {
        let type_id = unsafe { transmute::<TypeId, (u64, u64)>(TypeId::of::<Self>()).1 as usize };
        let group = type_id % G;
        cache.groups[group].load(Self::load_or_default(), type_id);
    }
    /// Retrieve Cachable from the cache.
    fn retrieve<const G: usize, const L: usize, const B: usize>(
        cache: &mut Cache<G, L, B>,
    ) -> CacheResult<&Self> {
        todo!()
    }
}

macro_rules! impl_cacheable_for_num {
    ($($t: ty),+) => {
        $(impl Cacheable for $t {
            fn load() -> CacheResult<Self> {
                Ok(0)
            }

            fn store(&self) -> CacheResult<()> {
                todo!()
            }
        })+
    };
}

impl_cacheable_for_num!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache() {
        let mut cache: Cache<1, 1, 512> = Default::default();
        cache.load::<i8>();
        cache.load::<i16>();
        cache.load::<i32>();
        cache.load::<i64>();
        cache.load::<isize>();
        cache.load::<u8>();
        cache.load::<u16>();
        cache.load::<u32>();
        cache.load::<u64>();
        cache.load::<usize>();
    }
}
