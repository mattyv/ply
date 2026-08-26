//! A component declaring a default `checks:` for the functions inside it
//! (The-Ply-Spec.md §5.1: "optional default checks for all fns in scope").
//!
//! `check` resolved that default and `verify` ignored it, so the same
//! document had two answers: the validating command reported these
//! functions as claiming `fuzz(64)`, and the run proved them with the
//! model checker instead -- a different check, a different meaning, and
//! nothing saying the declared one had been passed over.
//!
//! All three functions have the same contract, so any difference in what
//! they earn comes from the document rather than from the code.

#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn takes_the_default(x: u32) -> u32 {
    x + 1
}

#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn writes_its_own(x: u32) -> u32 {
    x + 1
}

#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn nested_takes_the_default(x: u32) -> u32 {
    x + 1
}
