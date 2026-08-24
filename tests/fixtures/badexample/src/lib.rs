//! Regression fixture for the 2026-08-24 M4 review's D1: a user's
//! `examples` entry that does not type-check makes the generated harness
//! crate fail to compile, so neither the `fuzz` nor the `test` check runs a
//! single case. The function itself is correct -- the defect under test is
//! entirely in what Ply reports when its own harness never built.
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

#[ply::requires(x < 1000 && y < 1000)]
#[ply::ensures(|result| *result == x + y)]
pub fn add_small(x: u32, y: u32) -> u32 {
    x + y
}
