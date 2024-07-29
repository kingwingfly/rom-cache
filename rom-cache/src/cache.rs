//! Cache data structure

use crate::error::CacheResult;
use crate::CacheError;

#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
#[cfg(loom)]
use loom::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::mem::transmute;
use std::ops::{Deref, DerefMut};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A cache storage structure.
/// - G: the number of cache groups
/// - L: the number of cache lines in each group
///
/// load a Cacheable into memory:
/// 1. cache hit: update LRU
/// 2. cache empty: load Cacheable into the cache
/// 3. cache group full: evict the least recently used Cacheable
/// 4. `Cacheable::load()` failed: use default value
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
    lock: Mutex<()>,
}

impl<const L: usize> Default for CacheGroup<L> {
    fn default() -> Self {
        let lines = (0..L).map(|_| CacheLine::default()).collect::<Vec<_>>();
        Self {
            lines: lines.try_into().unwrap(),
            lock: Mutex::new(()),
        }
    }
}

impl<const L: usize> CacheGroup<L> {
    /// load Cacheable into CacheLine and update LRU
    fn load<T: CacheableExt>(&self) -> CacheResult<usize> {
        let slot = self.slot::<T>();
        match slot {
            Some(CacheSlot::Hit(i)) => {
                let lru = self.lines[i].lru.load(Ordering::Acquire);
                self.lines
                    .iter()
                    .filter(|l| l.lru.load(Ordering::Acquire) < lru)
                    .for_each(|l| {
                        l.lru.fetch_add(1, Ordering::AcqRel);
                    });
                self.lines[i].lru.store(0, Ordering::Release);
                Ok(i)
            }
            Some(CacheSlot::Empty(i)) => {
                self.lines.iter().for_each(|l| {
                    l.lru.fetch_add(1, Ordering::AcqRel);
                });
                self.lines[i].lru.store(0, Ordering::Release);
                *self.lines[i].inner.write().unwrap() = Some(Box::new(T::load_or_default()));
                self.lines[i]
                    .type_id
                    .store(T::type_id_usize(), Ordering::Release);
                Ok(i)
            }
            Some(CacheSlot::Evict(i)) => {
                self.lines.iter().for_each(|l| {
                    l.lru.fetch_add(1, Ordering::AcqRel);
                });
                self.lines[i].lru.store(0, Ordering::Release);
                match self.lines[i].inner.try_write() {
                    Ok(mut guard) => {
                        if self.lines[i].dirty.swap(false, Ordering::AcqRel) {
                            guard.take().unwrap().store()?;
                        }
                        *guard = Some(Box::new(T::load_or_default()));
                        self.lines[i]
                            .type_id
                            .store(T::type_id_usize(), Ordering::Release);
                        Ok(i)
                    }
                    Err(_) => Err(CacheError::Busy),
                }
            }
            None => unreachable!(),
        }
    }

    fn slot<T: CacheableExt>(&self) -> Option<CacheSlot> {
        let type_id = T::type_id_usize();
        let mut slot = None;
        for (i, line) in self.lines.iter().enumerate() {
            if line.type_id.load(Ordering::Acquire) == type_id {
                return Some(CacheSlot::Hit(i));
            } else if line.type_id.load(Ordering::Acquire) == 0 {
                return Some(CacheSlot::Empty(i));
            } else if line.lru.load(Ordering::Acquire) as usize == L - 1 {
                slot = Some(CacheSlot::Evict(i));
            }
        }
        slot
    }

    /// Retrieve a Cacheable from the cache.
    fn retrieve<T: CacheableExt>(&self) -> CacheResult<CacheRef<'_, T>> {
        let _lock = self.lock.lock();
        let i = self.load::<T>()?;
        self.lines[i]
            .inner
            .try_read()
            .map(|guard| CacheRef {
                guard,
                _phantom: PhantomData,
            })
            .map_err(|_| CacheError::Locked)
    }

    /// Retrieve a mut Cacheable from the cache.
    fn retrieve_mut<T: CacheableExt>(&self) -> CacheResult<CacheMut<'_, T>> {
        let _lock = self.lock.lock();
        let i = self.load::<T>()?;
        self.lines[i]
            .inner
            .try_write()
            .map(|guard| CacheMut {
                guard,
                dirty: Some(self.lines[i].dirty.clone()),
                _phantom: PhantomData,
            })
            .map_err(|_| CacheError::Locked)
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
            .field("dirty", &self.dirty.load(Ordering::Acquire))
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
        let dyn_any: &dyn Any = &**self.guard.as_ref().unwrap();
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
        let dyn_any: &dyn Any = &**self.guard.as_ref().unwrap();
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
        let dyn_any: &mut dyn Any = &mut **self.guard.as_mut().unwrap();
        #[cfg(not(feature = "nightly"))]
        let dyn_any = self.guard.as_mut().unwrap().as_any_mut();
        dyn_any.downcast_mut::<T>().expect("downcast failed")
    }
}

#[derive(Debug)]
enum CacheSlot {
    Hit(usize),
    Empty(usize),
    Evict(usize),
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
    /// fn as_any_mut(&mut self) -> &mut dyn Any {
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

impl_cacheable_for_num!(i8, i16, i32, i64, isize);
impl_cacheable_for_num!(u8, u16, u32, u64, usize);
impl_cacheable_for_num!(String, Vec<u8>, Vec<u16>, Vec<u32>, Vec<u64>, Vec<usize>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache() {
        let cache: CacheInner<2, 2> = Default::default();
        cache.get::<isize>().unwrap();
        cache.get::<String>().unwrap();
        {
            let mut s = cache.get_mut::<String>().unwrap();
            cache.get::<u64>().unwrap();
            cache.get::<usize>().unwrap();
            *s = "".to_string();
        }
        {
            let s = cache.get::<String>().unwrap();
            assert_eq!(*s, "");
        }
    }

    #[test]
    #[cfg_attr(not(loom), ignore = "this is loom only test")]
    fn loom_test() {
        use loom::thread;
        loom::model(|| {
            let cache: Cache<1, 2> = Cache::default();

            let jhs: Vec<_> = (0..2)
                .map(|_| {
                    let cache = cache.clone();
                    thread::spawn(move || {
                        cache.get::<isize>().ok();
                        cache.get::<String>().ok();
                        {
                            let s = cache.get_mut::<String>();
                            cache.get::<u64>().ok();
                            cache.get::<usize>().ok();
                            if let Ok(mut s) = s {
                                *s = "".to_string();
                            }
                        }
                        {
                            if let Ok(s) = cache.get::<String>() {
                                assert_eq!(*s, "");
                            }
                        }
                    })
                })
                .collect();
            for jh in jhs {
                jh.join().unwrap();
            }
        });
    }
}
