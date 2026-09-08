//! `helperspec`'s shape with the helper in its **own file**, which is how
//! Rust is ordinarily written and where the same false clean survived
//! (2026-09-08, external review of `7820a4b`).
//!
//! `cargo mutants` names the function that owns a mutant by its path within
//! its own file, because the rest of the path is carried by the file name
//! printed beside it. So the helper below is `doubled_then_capped`, while
//! its inline-module twin in `helperspec` is `maths::doubled_then_capped`.
//! Ply named both the second way, so on this shape the selector matched
//! nothing, no bug was ever planted in the helper, and the run still
//! reported that every planted bug was caught.
//!
//! Everything else is `helperspec`, deliberately: same thin shell, same
//! bound-shaped promise that cannot see a wrong answer under the cap, and
//! no worked example, so mutation testing is the only instrument that can
//! see the bug.

mod maths;

/// The claimed function: a thin shell, exactly as the guide advises.
#[ply::requires(x < 1000)]
#[ply::ensures(|result| *result <= 100)]
pub fn scaled(x: u32) -> u32 {
    maths::doubled_then_capped(x)
}
