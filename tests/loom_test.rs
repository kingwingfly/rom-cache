//! This test need nightly Rust for feature `nightly` of `rom_cache` is enabled.
#![cfg(loom)]

use loom::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use rom_cache::{Cache, Cacheable};

loom::thread_local!(
    static NUM1: AtomicUsize = AtomicUsize::new(0);
    static NUM2: AtomicIsize = AtomicIsize::new(0);
);

#[derive(Default)]
struct Usize {
    inner: usize,
}

#[derive(Default)]
struct Isize {
    inner: isize,
}

impl Cacheable for Usize {
    fn load() -> std::io::Result<Self>
    where
        Self: Sized,
    {
        let inner = NUM1.with(|n| n.load(Ordering::Acquire));
        Ok(Self { inner })
    }

    fn store(&self) -> std::io::Result<()> {
        NUM1.with(|n| n.store(self.inner, Ordering::Release));
        Ok(())
    }
}

impl Cacheable for Isize {
    fn load() -> std::io::Result<Self>
    where
        Self: Sized,
    {
        let inner = NUM2.with(|n| n.load(Ordering::Acquire));
        Ok(Self { inner })
    }

    fn store(&self) -> std::io::Result<()> {
        NUM2.with(|n| n.store(self.inner, Ordering::Release));
        Ok(())
    }
}

#[test]
#[cfg_attr(not(loom), ignore = "loom only test")]
fn loom_test() -> core::result::Result<(), Box<dyn std::error::Error>> {
    loom::model(|| {
        let cache: Cache<1, 1> = Cache::default();
        {
            let mut a = cache.get_mut::<Usize>().unwrap();
            assert_eq!(NUM1.with(|n| n.load(Ordering::Relaxed)), 0);
            assert_eq!(a.inner, 0);
            a.inner += 1;
            assert_eq!(NUM1.with(|n| n.load(Ordering::Relaxed)), 0);
            assert_eq!(a.inner, 1);
        }
        {
            let b = cache.get::<Isize>().unwrap();
            assert_eq!(NUM1.with(|n| n.load(Ordering::Relaxed)), 1);
            assert_eq!(b.inner, 0);
        }
        assert_eq!(NUM1.with(|n| n.load(Ordering::Acquire)), 1);
        assert_eq!(NUM2.with(|n| n.load(Ordering::Acquire)), 0);
    });
    Ok(())
}
