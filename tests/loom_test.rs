//! This test need nightly Rust for feature `nightly` of `rom_cache` is enabled.
#![cfg(loom)]

use loom::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use loom::thread;
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
fn loom_test1() -> core::result::Result<(), Box<dyn std::error::Error>> {
    loom::model(|| {
        {
            let cache: Cache<1, 2> = Cache::default();
            let mut a = cache.get_mut::<Usize>().unwrap();
            a.inner += 1;
            let b = cache.get::<Isize>().unwrap();
            assert_eq!(b.inner, 0);
        }
        assert_eq!(NUM1.with(|n| n.load(Ordering::Relaxed)), 1);
        assert_eq!(NUM2.with(|n| n.load(Ordering::Relaxed)), 0);
        {
            let cache: Cache<1, 1> = Cache::default();
            {
                let mut a = cache.get_mut::<Usize>().unwrap();
                a.inner += 1;
            }
            {
                let b = cache.get::<Isize>().unwrap();
                assert_eq!(b.inner, 0);
            }
            assert_eq!(NUM1.with(|n| n.load(Ordering::Relaxed)), 2);
        }
    });
    Ok(())
}

#[test]
#[cfg_attr(not(loom), ignore = "loom only test")]
fn loom_test2() -> core::result::Result<(), Box<dyn std::error::Error>> {
    loom::model(|| {
        let cache: Cache<1, 1> = Cache::default();
        let ths: Vec<_> = (0..2)
            .map(|_| {
                let cache_c = cache.clone();
                thread::spawn(move || {
                    let _ = cache_c.get::<Usize>().unwrap();
                    let _ = cache_c.get::<Isize>().unwrap();
                })
            })
            .collect();

        for th in ths {
            th.join().unwrap();
        }
    });
    Ok(())
}

#[test]
#[cfg_attr(not(loom), ignore = "loom only test")]
fn loom_test3() -> core::result::Result<(), Box<dyn std::error::Error>> {
    loom::model(|| {
        let cache: Cache<1, 1> = Cache::default();
        let ths: Vec<_> = (0..2)
            .map(|_| {
                let cache_c = cache.clone();
                thread::spawn(move || {
                    let _ = cache_c.get::<Usize>().unwrap();
                })
            })
            .collect();

        for th in ths {
            th.join().unwrap();
        }
    });
    Ok(())
}

#[test]
#[cfg_attr(not(loom), ignore = "loom only test")]
fn loom_test4() -> core::result::Result<(), Box<dyn std::error::Error>> {
    loom::model(|| {
        let cache: Cache<1, 1> = Cache::default();
        let cache_c = cache.clone();
        let t1 = thread::spawn(move || {
            let _ = cache_c.get_mut::<Usize>().unwrap();
        });
        let cache_c = cache.clone();
        let t2 = thread::spawn(move || {
            let _ = cache_c.get_mut::<Isize>().unwrap();
        });
        t1.join().unwrap();
        t2.join().unwrap();
    });
    Ok(())
}

#[test]
#[cfg_attr(not(loom), ignore = "loom only test")]
#[should_panic(expected = "Locked")]
fn loom_test5() {
    loom::model(|| {
        let cache: Cache<1, 1> = Cache::default();
        let _a = cache.get_mut::<Usize>().unwrap();
        let _b = cache.get::<Usize>().unwrap();
    });
}

#[test]
#[cfg_attr(not(loom), ignore = "loom only test")]
#[should_panic(expected = "Locked")]
fn loom_test6() {
    loom::model(|| {
        let cache: Cache<1, 1> = Cache::default();
        let _a = cache.get_mut::<Usize>().unwrap();
        let _b = cache.get_mut::<Usize>().unwrap();
    });
}

#[test]
#[cfg_attr(not(loom), ignore = "loom only test")]
#[should_panic(expected = "Busy")]
fn loom_test7() {
    loom::model(|| {
        let cache: Cache<1, 1> = Cache::default();
        let _a = cache.get::<Usize>().unwrap();
        let _b = cache.get::<Isize>().unwrap();
    });
}
