//! Regression fixture (misattribution fix, 2026-08-26): two claimed
//! functions, sharing one generated harness crate (§5.4c), both completely
//! correct. The fix that stopped one broken function from taking its
//! crate-mates down with it must not add any new compile, noise, or
//! diagnostic to the case where nothing is broken at all.

#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result >= x)]
pub fn increment(x: u32) -> u32 {
    x + 1
}

#[ply::ensures(|result| *result >= x && *result >= y)]
pub fn add_small(x: u32, y: u32) -> u32 {
    x.saturating_add(y)
}
