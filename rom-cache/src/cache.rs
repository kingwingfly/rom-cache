//! Cache data structure

use crate::error::CacheResult;
use crate::CacheError;

#[cfg(loom)]
use loom::sync::{Arc, Mutex};
use std::any::{Any, TypeId};
#[cfg(not(loom))]
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::transmute;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::sync::{Arc, Mutex};

/// A cache storage structure.
/// - G: the number of cache groups
/// - L: the number of cache lines in each group
///
/// load a Cacheable into memory:
/// 1. cache hit: update LRU
/// 2. cache empty: load Cacheable into the cache
/// 3. cache group full: evict the least recently used Cacheable
/// 4. `Cacheable::load()` failed: IoError
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
    /// Retrieve a Cacheable from the cache. Use Default if `Cacheable::load()` failed.
    /// At most (usize::MAX >> 2) CacheRefs for **each** Cacheable type can be retrieved at the same time,
    /// or the counter will overflow and wrap-around, leading to a wrong state.
    /// - If the cache hit and is readable (i.e not being written), return a `CacheRef`. Use Default if `Cacheable::load()` failed.
    /// - CacheError::Busy: cache miss, the CacheLine chosen to evict is being used.
    /// - CacheError::Locked: cache hit, but the CacheLine for T is being written.
    pub fn get<T: Cacheable + Default>(&self) -> CacheResult<CacheRef<'_, T>> {
        self.inner.get::<T>()
    }

    /// Retrieve a mut Cacheable from the cache.
    /// At most 1 CacheMut for **each** Cacheable type can be retrieved at the same time.
    /// - If the cache hit and is writable (i.e not being read or written), return a `CacheMut`. Use Default if `Cacheable::load()` failed.
    /// - CacheError::Busy: cache miss, the CacheLine chosen to evict is being used.
    /// - CacheError::Locked: cache hit, but the CacheLine for T is being read or written.
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
    lines: UnsafeCell<[CacheLine; L]>,
    flags: UnsafeCell<[Flag; L]>,
    lock: Mutex<()>,
}

/// # Safety
/// All APIs of `CacheGroup` synchronize with the lock.
unsafe impl<const L: usize> Sync for CacheGroup<L> {}

impl<const L: usize> Default for CacheGroup<L> {
    fn default() -> Self {
        let lines = (0..L).map(|_| CacheLine::default()).collect::<Vec<_>>();
        let flags = (0..L).map(|_| Flag::default()).collect::<Vec<_>>();
        Self {
            lines: UnsafeCell::new(lines.try_into().unwrap()),
            flags: UnsafeCell::new(flags.try_into().unwrap()),
            lock: Mutex::new(()),
        }
    }
}

impl<const L: usize> Drop for CacheGroup<L> {
    fn drop(&mut self) {
        let lines = unsafe { &mut *self.lines.get() };
        let flags = unsafe { &*self.flags.get() };
        for (i, f) in flags.iter().enumerate() {
            if f.is_dirty() {
                lines[i].inner.take().unwrap().store().ok();
            }
        }
    }
}

impl<const L: usize> CacheGroup<L> {
    /// load Cacheable into CacheLine and update LRU
    fn load<T: CacheableExt + Default>(&self) -> CacheResult<usize> {
        let slot = self.slot::<T>();
        let lines = unsafe { &mut *self.lines.get() };
        let flags = unsafe { &*self.flags.get() };
        match slot {
            Some(CacheSlot::Hit(i)) => {
                let lru = lines[i].lru;
                lines
                    .iter_mut()
                    .filter(|l| l.lru < lru)
                    .for_each(|l| l.lru += 1);
                lines[i].lru = 0;
                Ok(i)
            }
            Some(CacheSlot::Empty(i)) => {
                lines.iter_mut().for_each(|l| l.lru += 1);
                lines[i].lru = 0;
                lines[i].inner = Some(Box::new(T::load_or_default()));
                lines[i].type_id = T::type_id_usize();
                Ok(i)
            }
            Some(CacheSlot::Evict(i)) => {
                lines.iter_mut().for_each(|l| l.lru += 1);
                lines[i].lru = 0;
                if !flags[i].in_using() {
                    if flags[i].is_dirty() {
                        lines[i].inner.take().unwrap().store()?;
                        flags[i].set_clean();
                    }
                    lines[i].inner = Some(Box::new(T::load_or_default()));
                    lines[i].type_id = T::type_id_usize();
                    Ok(i)
                } else {
                    Err(CacheError::Busy)
                }
            }
            None => unreachable!(),
        }
    }

    fn slot<T: CacheableExt>(&self) -> Option<CacheSlot> {
        let type_id = T::type_id_usize();
        let mut slot = None;
        let lines = unsafe { &*self.lines.get() };
        for (i, line) in lines.iter().enumerate() {
            if line.type_id == type_id {
                return Some(CacheSlot::Hit(i));
            } else if line.type_id == 0 {
                return Some(CacheSlot::Empty(i));
            } else if line.lru == L - 1 {
                slot = Some(CacheSlot::Evict(i));
            }
        }
        slot
    }

    /// Retrieve a Cacheable from the cache.
    /// At most 63 CacheRefs for each Cacheable type can be retrieved at the same time
    fn retrieve<T: CacheableExt + Default>(&self) -> CacheResult<CacheRef<'_, T>> {
        let _lock = self.lock.lock().map_err(|_| CacheError::Poisoned)?;
        let i = self.load::<T>()?;
        let lines = unsafe { &*self.lines.get() };
        let flags = unsafe { &*self.flags.get() };
        flags[i].read()?;
        let inner = lines[i].inner.as_deref().unwrap();
        let flag = &flags[i];
        Ok(CacheRef {
            inner,
            flag,
            _phantom: PhantomData,
        })
    }

    /// Retrieve a mut Cacheable from the cache.
    fn retrieve_mut<T: CacheableExt + Default>(&self) -> CacheResult<CacheMut<'_, T>> {
        let _lock = self.lock.lock().map_err(|_| CacheError::Poisoned)?;
        let i = self.load::<T>()?;
        let lines = unsafe { &mut *self.lines.get() };
        let flags = unsafe { &*self.flags.get() };
        flags[i].write()?;
        let inner = lines[i].inner.as_deref_mut().unwrap();
        let flag = &flags[i];
        Ok(CacheMut {
            inner,
            flag,
            _phantom: PhantomData,
        })
    }
}

#[derive(Debug)]
enum CacheSlot {
    Hit(usize),
    Empty(usize),
    Evict(usize),
}

#[derive(Default)]
struct CacheLine {
    lru: usize,
    type_id: usize,
    inner: Option<Box<dyn Cacheable>>,
}

impl std::fmt::Debug for CacheLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheLine")
            .field("lru", &self.lru)
            .field("type_id", &self.type_id)
            .finish()
    }
}

#[derive(Debug, Default)]
struct Flag {
    // 000...00
    //        ^ write
    //  ^^^^^^ read count
    // ^ dirty
    inner: AtomicUsize,
}

impl Flag {
    fn write(&self) -> CacheResult<()> {
        if self.inner.load(Ordering::Relaxed) == 0 {
            self.inner.store(1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(CacheError::Locked)
        }
    }

    fn read(&self) -> CacheResult<()> {
        if self.inner.load(Ordering::Relaxed) & 1 != 1 {
            self.inner.fetch_add(2, Ordering::Relaxed);
            Ok(())
        } else {
            Err(CacheError::Locked)
        }
    }

    fn end_write(&self) {
        self.inner.fetch_and(usize::MAX - 1, Ordering::Relaxed);
    }

    fn end_read(&self) {
        self.inner.fetch_sub(2, Ordering::Relaxed);
    }

    fn is_dirty(&self) -> bool {
        self.inner.load(Ordering::Relaxed) & !(usize::MAX >> 1) != 0
    }

    fn set_dirty(&self) {
        self.inner.fetch_or(!(usize::MAX >> 1), Ordering::Relaxed);
    }

    fn set_clean(&self) {
        self.inner.fetch_and(usize::MAX >> 1, Ordering::Relaxed);
    }

    fn in_using(&self) -> bool {
        self.inner.load(Ordering::Relaxed) & (usize::MAX >> 1) != 0
    }
}

/// An immutable ref wrapper to a cacheable object.
///
/// `Cache::get_mut::<T>()` will return `CacheError::Locked` before this ref dropped.
pub struct CacheRef<'a, T>
where
    T: Any,
{
    inner: &'a dyn Cacheable,
    flag: &'a Flag,
    _phantom: PhantomData<&'a T>,
}

impl<T: Any> Deref for CacheRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        #[cfg(feature = "nightly")]
        let dyn_any: &dyn Any = self.inner;
        #[cfg(not(feature = "nightly"))]
        let dyn_any = self.inner.as_any();
        dyn_any.downcast_ref::<T>().expect("downcast failed")
    }
}

impl<T: Any> Drop for CacheRef<'_, T> {
    fn drop(&mut self) {
        self.flag.end_read();
    }
}

/// A mutable ref wrapper to a cacheable object.
///
/// `Cache::get::<T>()` and `Cache::get_mut::<T>()`
/// will return `CacheError::Locked` before this mut ref dropped.
pub struct CacheMut<'a, T>
where
    T: Any,
{
    inner: &'a mut dyn Cacheable,
    flag: &'a Flag,
    _phantom: PhantomData<&'a T>,
}

impl<T: Any> Deref for CacheMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        #[cfg(feature = "nightly")]
        let dyn_any: &dyn Any = self.inner;
        #[cfg(not(feature = "nightly"))]
        let dyn_any = self.inner.as_any();
        dyn_any.downcast_ref::<T>().expect("downcast failed")
    }
}

impl<T: Any> DerefMut for CacheMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.flag.set_dirty();
        #[cfg(feature = "nightly")]
        let dyn_any: &mut dyn Any = self.inner;
        #[cfg(not(feature = "nightly"))]
        let dyn_any = self.inner.as_any_mut();
        dyn_any.downcast_mut::<T>().expect("downcast failed")
    }
}

impl<T: Any> Drop for CacheMut<'_, T> {
    fn drop(&mut self) {
        self.flag.end_write();
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
    /// fn as_any_mut(&mut self) -> &mut dyn Any {
    ///     self
    /// }
    /// ```
    #[cfg(not(feature = "nightly"))]
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

trait CacheableExt: Cacheable + Sized {
    /// Load Cacheable from the storage, or return default value.
    fn load_or_default() -> Self
    where
        Self: Default,
    {
        Self::load().unwrap_or_default()
    }
    /// Get the lower 64 bit of Cacheable's TypeId.
    fn type_id_usize() -> usize {
        unsafe { transmute::<TypeId, (u64, u64)>(TypeId::of::<Self>()).1 as usize }
    }
    /// Retrieve Cacheable from the cache.
    fn retrieve_from<const G: usize, const L: usize>(
        cache: &CacheInner<G, L>,
    ) -> CacheResult<CacheRef<'_, Self>>
    where
        Self: Default,
    {
        let type_id = Self::type_id_usize();
        let group = type_id % G;
        cache.groups[group].retrieve()
    }
    /// Retrieve mut Cacheable from the cache.
    fn retrieve_mut_from<const G: usize, const L: usize>(
        cache: &CacheInner<G, L>,
    ) -> CacheResult<CacheMut<'_, Self>>
    where
        Self: Default,
    {
        let type_id = Self::type_id_usize();
        let group = type_id % G;
        cache.groups[group].retrieve_mut()
    }
}

impl<T> CacheableExt for T where T: Cacheable + Sized {}

#[cfg(loom)]
#[derive(Debug)]
struct UnsafeCell<T>(loom::cell::UnsafeCell<T>);
#[cfg(loom)]
impl<T> UnsafeCell<T> {
    fn new(data: T) -> Self {
        Self(loom::cell::UnsafeCell::new(data))
    }
    fn get(&self) -> *mut T {
        self.0.with_mut(|ptr| ptr)
    }
}
