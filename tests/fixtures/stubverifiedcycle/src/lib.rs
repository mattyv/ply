//! Fixture for D5's ordering rule and its fallback (The-Ply-Spec.md §5.5):
//! `f` and `g` call each other, so neither can be placed before the other
//! in the callees-before-callers order. A cycle cannot be ordered, so both
//! fall back to the second branch (the callee's own contract is assumed,
//! never its body) -- staying `conditional`, never an error, never a hang.
//!
//! The contracts use exact equality (`result == x + 1`) rather than an
//! inequality: an assumed contract lets Kani pick *any* value satisfying
//! it, including one that would overflow a looser bound, which is a real
//! defect in a contract, not in this mechanism -- exact equality pins the
//! stood-in value to exactly one number, so it exists to exercise the
//! ordering/fallback rule, not to double as an overflow fixture.

#[ply::requires(x < 1000)]
#[ply::ensures(|result| *result == x + 1)]
pub fn g(x: u32) -> u32 {
    if x == 0 { 1 } else { f(x - 1) + 1 }
}

#[ply::requires(x < 1000)]
#[ply::ensures(|result| *result == x + 1)]
pub fn f(x: u32) -> u32 {
    if x == 0 { 1 } else { g(x - 1) + 1 }
}
