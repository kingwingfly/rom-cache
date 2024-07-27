#![doc = include_str!("../README.md")]
#![deny(
    missing_docs,
    rustdoc::broken_intra_doc_links,
    elided_lifetimes_in_paths
)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![cfg_attr(feature = "nightly", feature(trait_upcasting))]

pub mod cache;
pub mod error;

pub use cache::{Cache, Cacheable};
pub use error::*;
