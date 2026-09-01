//! docs/reach-measurement-2.md: a contract that names a type nowhere in the
//! checked fn's own signature -- not a parameter, not the return type, just
//! a name written in the `#[ply::ensures]` text -- used to fail the
//! generated harness to compile with `error[E0433]: cannot find type
//! Ordering in this scope`. The generated harness only ever imported the
//! checked fn's own path and the types its parameters/receiver walk found;
//! nothing walked the contract text itself.
//!
//! `is_le`'s own postcondition names `std::cmp::Ordering` directly, to say
//! what "less than or equal" means in terms of the type the standard
//! library itself uses to answer that question -- and the file above it
//! imports it exactly the ordinary way any Rust file would.

use std::cmp::Ordering;

#[ply::ensures(|result| *result == (a.cmp(&b) != Ordering::Greater))]
pub fn is_le(a: u32, b: u32) -> bool {
    a <= b
}
