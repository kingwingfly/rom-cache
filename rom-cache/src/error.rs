//! The error type for this crate.

use thiserror::Error;

/// The error type for this crate.
#[derive(Error, Debug)]
pub enum CacheError {
    /// IO error from [`Cacheable::load()`](crate::cache::Cacheable::load()) and [`Cacheable::store()`](crate::cache::Cacheable::store())
    #[error("Io error: {0}")]
    Io(#[from] std::io::Error),
    /// Cache is missing even loaded. This happens due to concurrency.
    #[error("Cache is missing.")]
    Missing,
    /// Lock poisoned due to LockGuard-holder panic.
    #[error("Lock poisoned")]
    Poisoned,
    /// The CacheLine chosen to evict is locked. Consider dropping lock you get, trying again or increasing the capacity of the cache.
    #[error("The CacheLine chosen to evict is locked. Consider dropping lock you get, trying again or increasing the capacity of the cache.")]
    Busy,
    /// The CacheLine is locked.
    #[error("The CacheLine is locked.")]
    Locked,
}

/// A specialized `Result` type for this crate.
pub type CacheResult<T> = std::result::Result<T, CacheError>;
