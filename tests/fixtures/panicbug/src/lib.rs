//! Regression fixture for the 2026-08-24 M4 review's D6: a function that
//! panics rather than returning makes the fuzz harness fail without ever
//! printing the failing-input marker Ply parses. Ply must report that it
//! could not produce a witness -- never a `violation` with nothing behind
//! it.
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

#[ply::ensures(|result| *result * 2 == x)]
pub fn halves(x: u32) -> u32 {
    if x % 2 == 1 {
        panic!("halves() only accepts even numbers");
    }
    x / 2
}
