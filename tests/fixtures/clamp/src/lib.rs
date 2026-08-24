//! Clamp fixture -- the D7 acceptance case (docs/plans/d7-replayable-tests.md),
//! taken verbatim from the M3 brief. Falsified at every x > 100. Nothing
//! panics anywhere -- the whole point is that Kani's own playback test stays
//! green here, and only Ply's rendered contract-assertion test can go red.
//!
//! This file is the pristine "before" state a user actually writes: the
//! function plus its contract, nothing else. `cargo-ply verify` generates
//! everything else (the Kani proof harness, the rendered cex test) at run
//! time -- the e2e tests exercise that against a scratch copy of this
//! fixture, never by hand-editing the checked-in source.

#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 {
    x.min(100)
}
