//! The same thin-wrapper shape as `helperspec`, with the helper one package
//! further away: a path dependency rather than another module.
//!
//! `reach` walks path dependencies, so the helper is correctly named as code
//! this claim's checks run. `cargo mutants` is invoked with `-p <root>`, so
//! until 2026-09-07 no deliberate bug was ever planted in it -- and the
//! report said the planting covered it. That is the `helperspec` defect one
//! package out (external review of cb8e3cd).

/// The claimed function: a thin shell, exactly as the guide advises.
#[ply::requires(x < 1000)]
#[ply::ensures(|result| *result <= 100)]
pub fn scaled(x: u32) -> u32 {
    helperpkg::doubled_then_capped(x)
}
