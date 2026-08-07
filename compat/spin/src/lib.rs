#![forbid(unsafe_code)]

//! Compatibility export for dependencies that still require yanked `spin 0.9`.
//! New code should depend on the maintained `spin` release directly.

pub use spin_next::*;
