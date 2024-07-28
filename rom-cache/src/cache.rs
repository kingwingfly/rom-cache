//! Cache data structure

use crate::error::CacheResult;
use crate::CacheError;

#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
#[cfg(loom)]
use loom::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::mem::transmute;
use std::ops::{Deref, DerefMut};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A cache storage structure.
/// - G: the number of cache groups
/// - L: the number of cache lines in each group
///
/// The cache refreshes itself by the LRU algorithm.
///
/// If [`CacheMut`] is dereferenced, cache will be marked dirty,
/// [`Cacheable::store()`] will be called when:
/// 1. The `Cache` is dropped.
/// 2. The `CacheLine` holding the dirty `Cacheable` is evicted.
#[derive(Default, Debug, Clone)]
pub struct Cache<const G: usize, const L: usize> {
    inner: Arc<CacheInner<G, L>>,
}

impl<const G: usize, const L: usize> Cache<G, L> {
    /// load a Cacheable into memory:
    /// 1. cache hit: update LRU
    /// 2. cache empty: load Cacheable into the cache
    /// 3. cache group full: evict the least recently used Cacheable
    /// 4. `Cacheable::load()` failed: use default value
    pub fn load<T: Cacheable + Default>(&self) -> CacheResult<()> {
        self.inner.load::<T>()
    }

    /// Retrieve a Cacheable from the cache.
    pub fn get<T: Cacheable + Default>(&self) -> CacheResult<CacheRef<'_, T>> {
        self.inner.get::<T>()
    }

    /// Retrieve a mut Cacheable from the cache.
    pub fn get_mut<T: Cacheable + Default>(&self) -> CacheResult<CacheMut<'_, T>> {
        self.inner.get_mut::<T>()
    }
}

#[derive(Debug)]
struct CacheInner<const G: usize, const L: usize> {
    groups: [CacheGroup<L>; G],
}

impl<const G: usize, const L: usize> Default for CacheInner<G, L> {
    fn default() -> Self {
        debug_assert!(G > 0, "Invalid number of cache groups {}.", G);
        debug_assert!(L > 0, "Invalid number of cache lines {}.", L);
        let groups = (0..G).map(|_| CacheGroup::default()).collect::<Vec<_>>();
        Self {
            groups: groups.try_into().unwrap(),
        }
    }
}

impl<const G: usize, const L: usize> CacheInner<G, L> {
    fn load<T: Cacheable + Default>(&self) -> CacheResult<()> {
        T::load_to(self)
    }

    fn get<T: Cacheable + Default>(&self) -> CacheResult<CacheRef<'_, T>> {
        T::retrieve_from(self)
    }

    fn get_mut<T: Cacheable + Default>(&self) -> CacheResult<CacheMut<'_, T>> {
        T::retrieve_mut_from(self)
    }
}

#[derive(Debug)]
struct CacheGroup<const L: usize> {
    lines: [CacheLine; L],
}

impl<const L: usize> Default for CacheGroup<L> {
    fn default() -> Self {
        let lines = (0..L).map(|_| CacheLine::default()).collect::<Vec<_>>();
        Self {
            lines: lines.try_into().unwrap(),
        }
    }
}

impl<const L: usize> CacheGroup<L> {
    /// load Cacheable into CacheLine and update LRU
    fn load<T: CacheableExt>(&self, type_id: usize) -> CacheResult<()> {
        let mut slug = (None, None, None);
        for (i, line) in self.lines.iter().enumerate() {
            if line.type_id.load(Ordering::Acquire) == type_id {
                slug.0 = Some(i); // hit
                continue;
            } else if slug.1.is_none() && line.inner.read().unwrap().is_none() {
                slug.1 = Some(i); // empty
                continue;
            } else if slug.1.is_none() && line.lru.load(Ordering::Acquire) as usize == L - 1 {
                slug.2 = Some(i); // expired
            }
        }
        match slug {
            // hit
            (Some(i), _, _) => {
                let lru = self.lines[i].lru.load(Ordering::Acquire);
                self.lines
                    .iter()
                    .filter(|l| l.lru.load(Ordering::Acquire) < lru)
                    .for_each(|l| {
                        l.lru.fetch_add(1, Ordering::AcqRel);
                    });
                self.lines[i].lru.store(0, Ordering::Release);
            }
            // empty
            (_, Some(i), None) => {
                self.lines.iter().for_each(|l| {
                    l.lru.fetch_add(1, Ordering::AcqRel);
                });
                self.lines[i].lru.store(0, Ordering::Release);
                *self.lines[i].inner.write().unwrap() = Some(Box::new(T::load_or_default()));
                self.lines[i].type_id.store(type_id, Ordering::Release);
            }
            //expired
            (_, _, Some(i)) => {
                self.lines.iter().for_each(|l| {
                    l.lru.fetch_add(1, Ordering::AcqRel);
                });
                self.lines[i].lru.store(0, Ordering::Release);
                let mut guard = self.lines[i].inner.write().unwrap();
                if self.lines[i].dirty.swap(false, Ordering::AcqRel) {
                    guard.take().unwrap().store()?;
                }
                *guard = Some(Box::new(T::load_or_default()));
                self.lines[i].type_id.store(type_id, Ordering::Release);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Retrieve a Cacheable from the cache.
    fn retrieve<T: CacheableExt>(&self) -> CacheResult<CacheRef<'_, T>> {
        let type_id = T::type_id_usize();
        self.lines
            .iter()
            .find(|l| l.type_id.load(Ordering::Acquire) == type_id)
            .map(|l| CacheRef {
                guard: l.inner.read().unwrap(),
                _phantom: PhantomData,
            })
            .ok_or(CacheError::Missing)
    }

    /// Retrieve a mut Cacheable from the cache.
    fn retrieve_mut<T: CacheableExt>(&self) -> CacheResult<CacheMut<'_, T>> {
        let type_id = T::type_id_usize();
        self.lines
            .iter()
            .find(|l| l.type_id.load(Ordering::Acquire) == type_id)
            .map(|l| CacheMut {
                guard: l.inner.write().unwrap(),
                dirty: Some(l.dirty.clone()),
                _phantom: PhantomData,
            })
            .ok_or(CacheError::Missing)
    }
}

#[derive(Default)]
struct CacheLine {
    lru: AtomicU8,
    type_id: AtomicUsize,
    inner: RwLock<Option<Box<dyn Cacheable + Send + Sync>>>,
    dirty: Arc<AtomicBool>,
}

impl std::fmt::Debug for CacheLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheLine")
            .field("lru", &self.lru)
            .field("type_id", &self.type_id)
            .field("inner", &"RwLock".to_string())
            .field("dirty", &(self.dirty.load(Ordering::Acquire)))
            .finish()
    }
}

impl Drop for CacheLine {
    fn drop(&mut self) {
        if self.dirty.load(Ordering::Acquire) {
            self.inner
                .write()
                .unwrap()
                .take()
                .unwrap()
                .store()
                .expect("Panic on storing dirty cache line");
        }
    }
}

/// A `RwLockReadGuard` wrapper to a cacheable object.
pub struct CacheRef<'a, T>
where
    T: Any,
{
    guard: RwLockReadGuard<'a, Option<Box<dyn Cacheable + Send + Sync>>>,
    _phantom: PhantomData<&'a T>,
}

impl<T: Any> Deref for CacheRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        #[cfg(feature = "nightly")]
        let dyn_any: &dyn Any = self.guard.as_ref().unwrap();
        #[cfg(not(feature = "nightly"))]
        let dyn_any = self.guard.as_ref().unwrap().as_any();
        dyn_any.downcast_ref::<T>().expect("downcast failed")
    }
}

/// A `RwLockWriteGuard` wrapper to a cacheable object.
pub struct CacheMut<'a, T>
where
    T: Any,
{
    guard: RwLockWriteGuard<'a, Option<Box<dyn Cacheable + Send + Sync>>>,
    dirty: Option<Arc<AtomicBool>>,
    _phantom: PhantomData<&'a T>,
}

impl<T: Any> Deref for CacheMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        #[cfg(feature = "nightly")]
        let dyn_any: &dyn Any = self.guard.as_ref().unwrap();
        #[cfg(not(feature = "nightly"))]
        let dyn_any = (self.guard.as_ref().unwrap()).as_any();
        dyn_any.downcast_ref::<T>().expect("downcast failed")
    }
}

impl<T: Any> DerefMut for CacheMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        if let Some(flag) = self.dirty.take() {
            flag.store(true, Ordering::Release);
        }
        #[cfg(feature = "nightly")]
        let dyn_any: &mut dyn Any = self.guard.as_mut().unwrap();
        #[cfg(not(feature = "nightly"))]
        let dyn_any = self.guard.as_mut().unwrap().as_any_mut();
        dyn_any.downcast_mut::<T>().expect("downcast failed")
    }
}

/// A type that can be cached.
pub trait Cacheable: Any + Send + Sync {
    /// Load Cacheable from the storage
    fn load() -> std::io::Result<Self>
    where
        Self: Sized;
    /// Write Cacheable back to storage.
    fn store(&self) -> std::io::Result<()>;

    /// As Any. This is needed since `Cacheable` will be used as `&dyn Cacheable`,
    /// and cannot upcast to `&dyn Any` in stable Rust. Just coding as following is Ok.
    /// ```ignore
    /// fn as_any(&self) -> &dyn Any {
    ///     self
    /// }
    /// ```
    /// Or you can simply enable `nightly` future, this needs nightly Rust.
    #[cfg(not(feature = "nightly"))]
    fn as_any(&self) -> &dyn Any;
    /// As Any mut.
    /// ```ignore
    /// fn as_any_mut(&self) -> &mut dyn Any {
    ///     self
    /// }
    /// ```
    #[cfg(not(feature = "nightly"))]
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

trait CacheableExt: Cacheable + Default {
    /// Load Cacheable from the storage to cache, or return the default value.
    fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }
    /// Get the lower 64 bit of Cacheable's TypeId.
    fn type_id_usize() -> usize {
        unsafe { transmute::<TypeId, (u64, u64)>(TypeId::of::<Self>()).1 as usize }
    }
    /// load Cacheable into the cache.
    fn load_to<const G: usize, const L: usize>(cache: &CacheInner<G, L>) -> CacheResult<()> {
        let type_id = Self::type_id_usize();
        let group = type_id % G;
        cache.groups[group].load::<Self>(type_id)
    }
    /// Retrieve Cacheable from the cache.
    fn retrieve_from<const G: usize, const L: usize>(
        cache: &CacheInner<G, L>,
    ) -> CacheResult<CacheRef<'_, Self>> {
        let type_id = Self::type_id_usize();
        let group = type_id % G;
        cache.groups[group].retrieve()
    }
    /// Retrieve mut Cacheable from the cache.
    fn retrieve_mut_from<const G: usize, const L: usize>(
        cache: &CacheInner<G, L>,
    ) -> CacheResult<CacheMut<'_, Self>> {
        let type_id = Self::type_id_usize();
        let group = type_id % G;
        cache.groups[group].retrieve_mut()
    }
}

impl<T> CacheableExt for T where T: Cacheable + Sized + Default {}

macro_rules! impl_cacheable_for_num {
    ($($t: ty),+) => {
        $(impl Cacheable for $t {
            fn load() -> std::io::Result<Self> {
                Ok(Self::default())
            }
            fn store(&self) -> std::io::Result<()> {
                Ok(())
            }
            #[cfg(not(feature = "nightly"))]
            fn as_any(&self) -> &dyn Any {
                self
            }
            #[cfg(not(feature = "nightly"))]
            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
        })+
    };
}

impl_cacheable_for_num!(
    i8,
    i16,
    i32,
    i64,
    isize,
    u8,
    u16,
    u32,
    u64,
    usize,
    String,
    Vec<u8>,
    Vec<u16>,
    Vec<u32>,
    Vec<u64>,
    Vec<usize>
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache() {
        let cache: CacheInner<2, 2> = Default::default();
        cache.load::<i32>().unwrap();
        cache.load::<i64>().unwrap();
        cache.load::<isize>().unwrap();
        cache.load::<String>().unwrap();
        {
            let mut s = cache.get_mut::<String>().unwrap();
            cache.load::<u32>().unwrap();
            cache.load::<u64>().unwrap();
            cache.load::<usize>().unwrap();
            *s = "hello, world.".to_string();
        }
        {
            let s = cache.get::<String>().unwrap();
            assert_eq!(*s, "hello, world.");
        }

        {
            let mut n = cache.get_mut::<isize>().unwrap();
            *n = 42;
        }
        {
            let n = cache.get::<usize>().unwrap();
            assert_eq!(*n, 0);
        }
        cache.load::<Vec<u8>>().unwrap();
        cache.load::<Vec<usize>>().unwrap();
    }

    #[test]
    #[cfg_attr(not(loom), ignore = "this is loom only test")]
    fn loom_test() {
        use loom::thread;

        loom::model(|| {
            let cache: Cache<2, 2> = Cache::default();

            let jhs: Vec<_> = (0..3)
                .map(|_| {
                    let cache = cache.clone();
                    thread::spawn(move || {
                        assert!(*cache.get::<i32>().unwrap() == 0);
                    })
                })
                .collect();
            for jh in jhs {
                jh.join().unwrap();
            }
        });
    }
}
