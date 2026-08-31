//! Defect 3 (2026-08-30, "a test that reproduces this" is false when two
//! functions fail): two fns, `seeded_bug_a` and `seeded_bug_b`, both break
//! their own promise for the same seeded input `x == 7` -- the fuzz
//! check's biased-small range (0..=16, weighted 3-to-1 over the full-range
//! arm) finds it for both essentially every run. `write_generated_test`
//! used to overwrite the whole `ply_generated_cex.rs` file on every call,
//! so calling it once per broken fn silently kept only the *last* one's
//! test while the terminal reported both as generated.
//!
//! Pristine "before" state: `cargo-ply verify` generates everything else
//! at run time.

#[ply::ensures(|result| *result == x)]
pub fn seeded_bug_a(x: u32) -> u32 {
    if x == 7 { x + 1 } else { x }
}

#[ply::ensures(|result| *result == y)]
pub fn seeded_bug_b(y: u32) -> u32 {
    if y == 7 { y + 1 } else { y }
}
