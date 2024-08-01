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
use std::sync::atomic::{AtomicU8, Ordering};
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
    /// Retrieve a Cacheable from the cache.
    pub fn get<T: Cacheable>(&self) -> CacheResult<CacheRef<'_, T>> {
        self.inner.get::<T>()
    }

    /// Retrieve a mut Cacheable from the cache.
    pub fn get_mut<T: Cacheable>(&self) -> CacheResult<CacheMut<'_, T>> {
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
    fn get<T: Cacheable>(&self) -> CacheResult<CacheRef<'_, T>> {
        T::retrieve_from(self)
    }

    fn get_mut<T: Cacheable>(&self) -> CacheResult<CacheMut<'_, T>> {
        T::retrieve_mut_from(self)
    }
}

#[derive(Debug)]
struct CacheGroup<const L: usize> {
    lines: UnsafeCell<[CacheLine; L]>,
    flags: UnsafeCell<[Flag; L]>,
    lock: Mutex<()>,
}

unsafe impl<const L: usize> Send for CacheGroup<L> {}
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
    fn load<T: CacheableExt>(&self) -> CacheResult<usize> {
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
                lines[i].inner = Some(Box::new(T::load()?));
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
                    lines[i].inner = Some(Box::new(T::load()?));
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
            } else if line.lru as usize == L - 1 {
                slot = Some(CacheSlot::Evict(i));
            }
        }
        slot
    }

    /// Retrieve a Cacheable from the cache.
    fn retrieve<T: CacheableExt>(&self) -> CacheResult<CacheRef<'_, T>> {
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
    fn retrieve_mut<T: CacheableExt>(&self) -> CacheResult<CacheMut<'_, T>> {
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

#[derive(Default)]
struct CacheLine {
    lru: u8,
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
    // 00000000
    //        ^ write
    //  ^^^^^^ read count
    // ^ dirty
    inner: AtomicU8,
}

impl Flag {
    fn write(&self) -> CacheResult<()> {
        if self
            .inner
            .compare_exchange_weak(0, 0b0000_0001, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            Ok(())
        } else {
            Err(CacheError::Locked)
        }
    }

    fn read(&self) -> CacheResult<()> {
        if self.inner.load(Ordering::Relaxed) & 1 != 1 {
            self.inner.fetch_add(0b0000_0010, Ordering::Relaxed);
            Ok(())
        } else {
            Err(CacheError::Locked)
        }
    }

    fn end_write(&self) {
        self.inner.fetch_and(0b1111_1110, Ordering::Relaxed);
    }

    fn end_read(&self) {
        self.inner.fetch_sub(0b0000_0010, Ordering::Relaxed);
    }

    fn is_dirty(&self) -> bool {
        self.inner.load(Ordering::Relaxed) & 0b1000_0000 != 0
    }

    fn set_dirty(&self) {
        self.inner.fetch_or(0b1000_0000, Ordering::Relaxed);
    }

    fn set_clean(&self) {
        self.inner.fetch_and(0b0111_1111, Ordering::Relaxed);
    }

    fn in_using(&self) -> bool {
        self.inner.load(Ordering::Relaxed) & 0b0111_1111 != 0
    }
}

/// A `RwLockReadGuard` wrapper to a cacheable object.
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

/// A `RwLockWriteGuard` wrapper to a cacheable object.
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

trait CacheableExt: Cacheable + Sized {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    impl_cacheable_for_num!(u8, u16, u32, u64, usize, String);

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
