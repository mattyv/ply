//! The shape Ply's own writing guide teaches -- lift the logic into a helper
//! and keep the claimed function a thin shell -- checked against the tier
//! Ply advertises for it.
//!
//! `spec-strong` means "mutation testing planted deliberate bugs and the
//! checks caught every one". Until 2026-09-07 the planting only ever touched
//! the *claimed function's own lines*, so on this shape it planted bugs in a
//! function containing no logic and reported that nothing survived. The
//! arithmetic below was never mutated at all.
//!
//! Found by handing a real task to an agent that followed the guide, then
//! planting a bug in its helper: the wait shrank on every attempt and the
//! run still came back `fuzzed(256)·spec-strong`.

/// Where the logic actually lives. Nothing claims this directly.
fn doubled_then_capped(x: u32) -> u32 {
    let doubled = x.saturating_mul(2);
    if doubled > 100 { 100 } else { doubled }
}

/// The claimed function: a thin shell, exactly as the guide advises.
#[ply::requires(x < 1000)]
#[ply::ensures(|result| *result <= 100)]
pub fn scaled(x: u32) -> u32 {
    doubled_then_capped(x)
}
