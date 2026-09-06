//! Fixture for The-Ply-Spec.md §5.2a: flags are part of the build and appear
//! nowhere in the source.
//!
//! `answer` compiles to a different body depending on whether `broken` is
//! set, and `RUSTFLAGS="--cfg broken"` sets it. Nothing in this file, in the
//! manifest, in the compiler version or in the target changes -- so before
//! 2026-09-06 the fingerprint was identical across the two builds and a pass
//! recorded under one was served under the other.
//!
//! Reported by external review, 2026-09-06.

/// Returns 7 normally and 8 under `--cfg broken`, so the promise below holds
/// in one build and fails in the other.
pub fn answer(x: u32) -> u32 {
    let cap = if cfg!(broken) { 8 } else { 7 };
    x.min(cap)
}
