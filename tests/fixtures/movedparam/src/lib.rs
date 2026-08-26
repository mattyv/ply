//! Regression fixture, the exact repro that found both 2026-08-26 defects.
//!
//! `vector`'s postcondition reads `v.len()` after `v: Vec<u8>` has already
//! been moved into the call (it is taken by value, not by reference) --
//! the generated harness could only compile that as `error[E0382]: borrow
//! of moved value: `v``. Ply now refuses this contract by name (`V0506`)
//! instead of generating code that cannot compile.
//!
//! Before the fix for the other defect found the same day, that one compile
//! failure was reported against *every* claim sharing this crate's one
//! generated harness (§5.4c) -- including `scalar`, which takes no `v` at
//! all and is otherwise completely correct.

/// Unrelated to `vector`, and correct. Must earn its own real verdict.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result >= x)]
pub fn scalar(x: u32) -> u32 {
    x + 1
}

/// `v` is taken by value, so it is moved into the call -- reading `v.len()`
/// afterward, with no `old(v)` around it, is a use-after-move.
#[ply::requires(v.len() <= 4)]
#[ply::ensures(|result| *result as usize >= v.len())]
pub fn vector(v: Vec<u8>) -> u32 {
    v.len() as u32
}
