use rom_cache::{Cache, Cacheable};

#[derive(Default, Debug)]
struct Data<const L: usize> {
    inner: usize,
}

impl<const L: usize> Cacheable for Data<L> {
    fn load() -> std::io::Result<Self>
    where
        Self: Sized,
    {
        Ok(Self { inner: 0 })
    }

    fn store(&self) -> std::io::Result<()> {
        Ok(())
    }
}

fn main() {
    let cache: Cache<1, 2> = Cache::default();
    let mut a = cache.get_mut::<Data<1>>().unwrap();
    a.inner = 1;
    let b = cache.get::<Data<2>>().unwrap();
    assert_ne!(a.inner, b.inner);
}
