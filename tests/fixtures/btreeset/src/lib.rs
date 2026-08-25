//! M4 acceptance fixture: the Kani-excluded shape (§5.4b: "BTreeSet ...
//! beyond a single element -- insert's own generic algorithm is
//! intractable at two elements even with the unwind fix applied") earning
//! an honest `fuzzed(n)` verdict -- proptest has no such limit, which is
//! the entire point of the M4 fuzz tier (§1).
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

use std::collections::BTreeSet;

#[ply::ensures(|result| *result == xs.len() as u32)]
pub fn count_unique(xs: &BTreeSet<u8>) -> u32 {
    xs.len() as u32
}
