//! M4 acceptance fixture: a seeded bug the fuzz check finds and shrinks to
//! a minimal counterexample, rendered through the same `contract_rt`
//! renderer the Kani (`bounded`) path uses (D7's "two consumers, one
//! renderer" design). `x == 7` sits inside the fuzz strategy's biased-small
//! range (0..=16, weighted 3-to-1 over the full-range arm), so 256 cases
//! finds it essentially every run.
//!
//! Pristine "before" state, same convention as the M3 fixtures: `cargo-ply
//! verify` generates everything else at run time.

#[ply::ensures(|result| *result == x)]
pub fn seeded_bug(x: u32) -> u32 {
    if x == 7 { x + 1 } else { x }
}
