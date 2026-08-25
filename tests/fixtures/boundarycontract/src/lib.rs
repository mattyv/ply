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

/// The caller. Its body is vetting 004's `tier_fee_cents` shape, deliberately
/// (2026-08-25): the defensive `.min` any caller of a table lookup writes, the
/// widened product, and the full `100_000_000` movement ceiling. A trivial
/// body would prove in ~10s and hide the thing this fixture now exists to
/// observe -- that a *stubbed* proof is expensive, because the stub hands
/// Kani a symbolic value where the real body returns one of four concrete
/// ones. At 60s (the old scalar default) this times out and says nothing
/// about the assumption; it is the tranche's headline capability, and it was
/// dead at the tool's own defaults.
#[ply::requires(amount_cents <= 100_000_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn tiered_fee(amount_cents: u32, tier: u8) -> u32 {
    let bps = legacy_rate(tier).min(10_000);
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}
