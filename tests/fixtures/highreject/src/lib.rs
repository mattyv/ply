//! Regression fixture for the 2026-08-24 M4 review's D3: a `requires` that
//! rejects roughly two thirds of the generated inputs must raise the
//! high-rejection warning §5.4c promises. The contract itself holds for
//! every accepted input -- the check under test is the warning, not a
//! violation.
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

#[ply::requires(x > 14)]
#[ply::ensures(|result| *result == x)]
pub fn mostly_rejected(x: u32) -> u32 {
    x
}
