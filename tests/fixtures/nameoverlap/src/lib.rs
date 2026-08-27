//! Blocker 2 (task 2026-08-27, docs/review-strings-receivers.md finding 2):
//! "a failing test is attributed to the wrong function." A top-level
//! `parse` and a `util::parse` -- ordinary name pairs, nothing exotic. The
//! generated harness module for `parse` is named `parse_harness`, and for
//! `util::parse` it is `util_parse_harness` -- and `parse_harness::` is a
//! *substring* of `util_parse_harness::`, so `cargo test`'s own plain-
//! substring filter for `parse` used to also execute `util::parse`'s own
//! tests, and Ply then blamed the correct `parse` for `util::parse`'s own
//! broken promise. `util::parse`'s promise is false on every input;
//! `parse`'s promise is true on every input its own precondition allows.

pub mod util {
    /// FALSE on every input: this returns `x`, not `0`.
    #[ply::ensures(|result| *result == 0)]
    pub fn parse(x: u32) -> u32 {
        x
    }
}

/// TRUE on every input: `parse` returns exactly `x`, which is always
/// `>= x`.
#[ply::ensures(|result| *result >= x)]
pub fn parse(x: u32) -> u32 {
    x
}
