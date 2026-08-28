//! Compatibility facade for the product-owned verdict kernel.
//!
//! The one authoritative implementation lives in `ply_core::kernel`; this
//! tooling crate keeps the exhaustive test workspace and existing imports
//! working while dependencies continue to point inward.

pub use ply_core::kernel::*;
