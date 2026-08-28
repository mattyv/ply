//! Regression fixture for the misattribution defect (2026-08-26): `fuzz`
//! and `test` checks in one crate share a single generated harness crate
//! (The-Ply-Spec.md §5.4c). Before this fixture's fix, a compile failure
//! *anywhere* in that shared crate was reported against every claim inside
//! it -- so `good_fn` here, which is completely correct, used to come back
//! `tool_error` quoting `bad_examples_fn`'s own compiler error about a
//! variable `good_fn` does not have.
//!
//! `bad_examples_fn`'s second `examples:` entry compares a `u32` against a
//! string literal (the same shape as the `badexample` fixture): it parses
//! as a Rust expression (§5.4a: examples are arbitrary Rust `==`
//! expressions, never type-checked before codegen) but cannot compile.

/// Completely fine. Must earn its own real verdict no matter what else in
/// this crate is broken.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result >= x)]
pub fn good_fn(x: u32) -> u32 {
    x + 1
}

#[ply::requires(x < 1000 && y < 1000)]
#[ply::ensures(|result| *result == x + y)]
pub fn bad_examples_fn(x: u32, y: u32) -> u32 {
    x + y
}
