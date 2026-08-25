//! Regression fixture for the 2026-08-24 M4 review's D7 and O4: the
//! milestone's own headline shape (`BTreeSet<u8>`, Kani-excluded per §5.4b)
//! with a seeded bug, so the witness-only `W0541` path runs for real
//! instead of by code reading. The bug fires exactly when the set contains
//! `3`, so the smallest failing input is the one-element set `[3]` -- which
//! also makes proptest's shrinking observable (the review's O2: the
//! `fuzzbug` fixture cannot detect its loss, because its bug fails at a
//! single input already).
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

use std::collections::BTreeSet;

#[ply::ensures(|result| *result == xs.len() as u32)]
pub fn count_unique(xs: &BTreeSet<u8>) -> u32 {
    xs.len() as u32 + if xs.contains(&3) { 1 } else { 0 }
}
