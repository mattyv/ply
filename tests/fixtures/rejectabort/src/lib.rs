//! Regression fixture for the 2026-08-24 M4 review's D4: a `requires` so
//! narrow that proptest abandons the run must not earn a verdict claiming
//! evidence it never gathered. The function and its contract are both
//! correct -- what is under test is the honesty of the verdict when the
//! engine gave up.
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

#[ply::requires(x > 20 && x < 24)]
#[ply::ensures(|result| *result == x)]
pub fn narrow_window(x: u32) -> u32 {
    x
}
