//! The error type for this crate.

use thiserror::Error;

/// The error type for this crate.
#[allow(missing_docs)]
#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Cache line is missing.")]
    Missing,
    #[error("RwLock poisoned.")]
    Poisoned,
}

/// A specialized `Result` type for this crate.
pub type CacheResult<T> = std::result::Result<T, CacheError>;
