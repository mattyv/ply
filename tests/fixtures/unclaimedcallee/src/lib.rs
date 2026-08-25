//! Fixture for D5's third branch (The-Ply-Spec.md §5.5, added 2026-08-25
//! after vetting 004): a contracted function whose body calls a function
//! nobody has vouched for. `legacy_rate` carries no contract -- inline or
//! in ply.yaml -- so Ply must refuse to descend into it rather than let
//! Kani inline the body and quietly fold it into the caller's `bounded`
//! verdict.

/// The unclaimed callee: ordinary code, no contract, nobody's promise.
pub fn legacy_rate(tier: u8) -> u32 {
    if tier == 0 { 150 } else { 90 }
}

#[ply::requires(amount_cents <= 100_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn tiered_fee(amount_cents: u32, tier: u8) -> u32 {
    ((amount_cents as u64 * legacy_rate(tier) as u64) / 10_000) as u32
}
