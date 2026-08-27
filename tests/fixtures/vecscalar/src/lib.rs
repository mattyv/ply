//! Acceptance fixture for `Vec<T>` on an already fuzz-supported scalar `T`
//! (task, 2026-08-27): `Vec<u32>` is fuzz-supported, never bounded-supported
//! -- this shape pre-dates this task (M4), but had no full end-to-end
//! (`cargo ply verify`) fixture until now, only unit-level coverage in
//! `harness.rs`/`fuzz_gen.rs`.

#[ply::ensures(|result| *result == xs.len() as u32)]
pub fn count(xs: &Vec<u32>) -> u32 {
    xs.len() as u32
}

#[ply::ensures(|result| *result == xs.len() as u32)]
pub fn count_bounded(xs: &Vec<u32>) -> u32 {
    xs.len() as u32
}

// A by-*value* `Vec<u8>` whose postcondition never reads `v` back (so it is
// not refused as a moved-parameter read, unlike the `movedparam` fixture's
// `vector`) -- the exact shape that exposed the marker-precompute defect
// this task's own fix closed generally, not only for `String`. Regression
// coverage for "Vec<u8> already exists for both engines; do not regress
// it" (task brief) on the fuzz side specifically.
#[ply::ensures(|result| *result <= 255u32 * 8)]
pub fn sum_moved(v: Vec<u8>) -> u32 {
    v.iter().map(|&b| b as u32).sum()
}
