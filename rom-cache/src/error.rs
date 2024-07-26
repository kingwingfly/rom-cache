//! The error type for this crate.

use thiserror::Error;

/// The error type for this crate.
#[derive(Error, Debug)]
pub enum CacheError {}

/// A specialized `Result` type for this crate.
pub type CacheResult<T> = std::result::Result<T, CacheError>;
