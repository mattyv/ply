//! Fixture for D5's *second* branch reached through a `ply.yaml`-declared
//! contract (The-Ply-Spec.md §5.5): `legacy_rate` carries no inline
//! contract, but ply.yaml declares one for it. Ply must assume that
//! contract, stand in for the callee inside the proof rather than descend
//! into it, and mark `tiered_fee`'s verdict `conditional` -- listing what
//! it assumed, and saying the assumption is owed evidence.

/// The boundary callee: no `ply::` attributes, exactly like two-year-old
/// code. Its contract lives in ply.yaml.
pub fn legacy_rate(tier: u8) -> u32 {
    if tier == 0 { 150 } else { 90 }
}

#[ply::requires(amount_cents <= 100_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn tiered_fee(amount_cents: u32, tier: u8) -> u32 {
    ((amount_cents as u64 * legacy_rate(tier) as u64) / 10_000) as u32
}
