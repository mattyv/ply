//! Fixture for The-Ply-Spec.md §5.2a: the first-party source set collected
//! only `src/`, and a build script is code the build runs.
//!
//! `answer` compiles against a value the build script emits. Rewriting the
//! script changes what `answer` returns while every line under `src/` stays
//! byte-identical, so the fingerprint did not move and a stored pass was
//! served over a function that now breaks its promise.
//!
//! Reported by external review, 2026-09-06.

/// The value comes from `build.rs`, through the environment Cargo sets for
/// this compilation. `env!` is a macro, which widens the scope to the whole
/// crate -- and the whole crate did not include the build script.
pub fn answer(x: u32) -> u32 {
    x.min(env!("PLY_FIXTURE_ANSWER").parse::<u32>().unwrap_or(0))
}
