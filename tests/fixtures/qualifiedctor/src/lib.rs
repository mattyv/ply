//! A constructor written in a **qualified** `impl` block, for a type used as
//! an ordinary parameter.
//!
//! The receiver path learned to resolve `impl super::T` / `impl crate::T` /
//! `impl Alias` back to the same declaration as `impl T` (2026-08-27, after
//! a mutator in a qualified `impl` was never called and the verdict claimed
//! coverage it did not have). The parameter path kept the older rule: an
//! `impl` block counts only when its self type is spelled as a single bare
//! segment. `Quota` below is spelled `super::Quota`, so its constructor was
//! invisible and Ply refused to build a value of a type it could build
//! perfectly well.
//!
//! Nothing here is unusual Rust. A type declared in `lib.rs` with its
//! methods in a submodule has to write `impl super::Quota`; there is no
//! other spelling available in that file.

pub struct Quota {
    per_second: u32,
}

pub mod ops;

/// Reads a field no caller can reach, so the value has to come from the
/// constructor rather than a struct literal.
#[ply::ensures(|result| *result >= 1)]
pub fn refill_per_second(q: Quota) -> u32 {
    q.per_second
}
