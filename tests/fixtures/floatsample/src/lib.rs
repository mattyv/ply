//! Acceptance fixture for the sampling/proving split (task, 2026-08-27):
//! `f32`/`f64` are the headline sample-only shape -- fuzz-supported, never
//! bounded-supported, and refused *by name* (not silently downgraded) when
//! `bounded` is asked for anyway.
//!
//! `increment` also carries the NaN/infinity decision's own real-world
//! stake: its postcondition (`result >= x`) holds for every *finite* `f64`
//! -- including the extremes, where `x + 1.0` rounds back to `x` exactly at
//! large enough magnitude, or overflows to `f64::INFINITY`, and `result >=
//! x` still holds either way -- but is false the moment `x` is `NaN`
//! (`NaN >= NaN` is `false`, like every NaN comparison). A clean verdict
//! here is only honest because Ply's default float sampling excludes NaN;
//! if that decision were reversed, this exact fixture would flip to a real
//! (but false) counterexample on some seed.

#[ply::ensures(|result| *result >= x)]
pub fn increment(x: f64) -> f64 {
    x + 1.0
}

#[ply::ensures(|result| *result == x)]
pub fn mirror32_bounded(x: f32) -> f32 {
    x
}

#[ply::ensures(|result| *result == x)]
pub fn mirror32_fuzzed(x: f32) -> f32 {
    x
}
