//! Cache data structure

use crate::error::CacheResult;
use crate::CacheError;

use std::any::{Any, TypeId};
use std::mem::{transmute, MaybeUninit};

/// A cache data structure.
/// - G: the number of cache groups
/// - L: the number of cache lines in each group
#[derive(Debug)]
pub struct Cache<const G: usize, const L: usize> {
    groups: [CacheGroup<L>; G],
}

impl<const G: usize, const L: usize> Default for Cache<G, L> {
    fn default() -> Self {
        debug_assert!(G > 0, "Invalid number of cache groups {}.", G);
        debug_assert!(L > 0, "Invalid number of cache lines {}.", L);
        unsafe { MaybeUninit::zeroed().assume_init() }
    }
}

impl<const G: usize, const L: usize> Cache<G, L> {
    /// load a Cacheable into memory
    pub fn load<T: Cacheable>(&mut self) {
        T::load_to(self);
    }

    /// Retrieve a Cacheable from the cache.
    pub fn retrieve<T: Cacheable>(&self) -> CacheResult<&T> {
        T::retrieve_from(self)
    }

    /// Retrieve a mut Cacheable from the cache.
    pub fn retrieve_mut<T: Cacheable>(&mut self) -> CacheResult<&mut T> {
        T::retrieve_mut_from(self)
    }
}

#[derive(Debug)]
struct CacheGroup<const L: usize> {
    lines: [CacheLine; L],
}

impl<const L: usize> Default for CacheGroup<L> {
    fn default() -> Self {
        debug_assert!(L > 0, "Invalid number of cache lines {}.", L);
        unsafe { MaybeUninit::zeroed().assume_init() }
    }
}

impl<const L: usize> CacheGroup<L> {
    /// load Cacheable as CacheLine into the cache.
    fn load<T: Cacheable>(&mut self, type_id: usize) {
        let mut slug = (None, None, None);
        for (i, line) in self.lines.iter().enumerate() {
            if line.type_id == type_id {
                slug.0 = Some(i); // hit
                continue;
            } else if slug.1.is_none() && line.inner.is_none() {
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
                self.lines[i].inner = Some(Box::new(T::load_or_default()));
            }
            // empty
            (_, Some(i), None) => {
                self.lines.iter_mut().for_each(|l| l.lru += 1);
                self.lines[i].lru = 0;
                self.lines[i].inner = Some(Box::new(T::load_or_default()));
                self.lines[i].type_id = type_id;
            }
            //expired
            (_, _, Some(i)) => {
                self.lines.iter_mut().for_each(|l| l.lru += 1);
                self.lines[i].lru = 0;
                if let Some(store_fn) = self.lines[i].store_fn.take() {
                    store_fn(self.lines[i].inner.take().unwrap()).ok();
                }
                self.lines[i].inner = Some(Box::new(T::load_or_default()));
                self.lines[i].type_id = type_id;
            }
            _ => unreachable!(),
        }
    }

    /// Retrieve Cacheable from the cache.
    fn retrieve<T: Cacheable>(&self) -> CacheResult<&T> {
        let type_id = T::type_id_usize();
        self.lines
            .iter()
            .find(|l| l.type_id == type_id)
            .and_then(|l| l.inner.as_deref().and_then(|b| b.downcast_ref()))
            .ok_or(CacheError::Missing)
    }

    /// Retrieve mut Cacheable from the cache.
    fn retrieve_mut<T: Cacheable>(&mut self) -> CacheResult<&mut T> {
        let type_id = T::type_id_usize();
        self.lines
            .iter_mut()
            .find(|l| l.type_id == type_id)
            .and_then(|l| {
                l.store_fn = Some(Box::new(|b| b.downcast_ref::<T>().unwrap().store()));
                l.inner.as_deref_mut().and_then(|b| b.downcast_mut())
            })
            .ok_or(CacheError::Missing)
    }
}

struct CacheLine {
    lru: u8,
    type_id: usize,
    inner: Option<Box<dyn Any>>,
    #[allow(clippy::type_complexity)]
    store_fn: Option<Box<dyn FnOnce(Box<dyn Any>) -> CacheResult<()>>>,
}

impl std::fmt::Debug for CacheLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheLine")
            .field("lru", &self.lru)
            .field("type_id", &self.type_id)
            .field("inner", &self.inner.is_some())
            .field("dirty", &self.store_fn.is_some())
            .finish()
    }
}

impl Default for CacheLine {
    fn default() -> Self {
        unsafe { MaybeUninit::zeroed().assume_init() }
    }
}

impl Drop for CacheLine {
    fn drop(&mut self) {
        if let Some(store_fn) = self.store_fn.take() {
            store_fn(self.inner.take().unwrap()).ok();
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
    /// Get the lower 64 bit of Cachable's TypeId.
    fn type_id_usize() -> usize {
        unsafe { transmute::<TypeId, (u64, u64)>(TypeId::of::<Self>()).1 as usize }
    }
    /// load Cachable into the cache.
    fn load_to<const G: usize, const L: usize>(cache: &mut Cache<G, L>) {
        let type_id = Self::type_id_usize();
        let group = type_id % G;
        cache.groups[group].load::<Self>(type_id);
    }
    /// Retrieve Cachable from the cache.
    fn retrieve_from<const G: usize, const L: usize>(cache: &Cache<G, L>) -> CacheResult<&Self> {
        let type_id = Self::type_id_usize();
        let group = type_id % G;
        cache.groups[group].retrieve()
    }
    /// Retrieve mut Cachable from the cache.
    fn retrieve_mut_from<const G: usize, const L: usize>(
        cache: &mut Cache<G, L>,
    ) -> CacheResult<&mut Self> {
        let type_id = Self::type_id_usize();
        let group = type_id % G;
        cache.groups[group].retrieve_mut()
    }
}

macro_rules! impl_cacheable_for_num {
    ($($t: ty),+) => {
        $(impl Cacheable for $t {
            fn load() -> CacheResult<Self> {
                Ok(0)
            }

            fn store(&self) -> CacheResult<()> {
                // println!("{}", std::any::type_name::<Self>());
                Ok(())
            }
        })+
    };
}

impl_cacheable_for_num!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl Cacheable for String {
    fn load() -> CacheResult<Self> {
        Ok("hello, world.".to_string())
    }

    fn store(&self) -> CacheResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache() {
        let mut cache: Cache<2, 2> = Default::default();
        cache.load::<String>();
        let s = cache.retrieve::<String>().unwrap();
        assert_eq!(s, "hello, world.");
        cache.load::<i8>();
        cache.load::<i16>();
        cache.load::<i32>();
        cache.load::<i64>();
        cache.load::<isize>();

        let n = cache.retrieve_mut::<isize>().unwrap();
        *n = 0;

        cache.load::<u8>();
        cache.load::<u16>();
        cache.load::<u32>();
        cache.load::<u64>();
        cache.load::<usize>();
        let n = cache.retrieve::<usize>().unwrap();
        assert_eq!(*n, 0);
    }
}
