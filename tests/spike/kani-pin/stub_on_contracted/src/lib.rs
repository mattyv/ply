//! Blocker 2: does `#[kani::stub]` compile when the *target* carries a
//! contract? Kani issue #4591 (open, filed 2026-04-23) says no at 0.67.0:
//! "Failed to find contract closure `__kani_recursion_check_rate`".
//!
//! This is Ply's D5 fallback shape. A callee that is only fuzzed or tested is
//! verified under an *assumed* contract, and vetting 004's `tier_fee_cents`
//! calls a contracted callee (`fee_cents`) as well as an unclaimed one, so a
//! stub over a contracted function is a shape Ply reaches, not a curiosity.
//!
//! `#[cfg_attr(kani, kani::ensures(..))]` is exactly what `#[ply::ensures]`
//! expands to (crates/ply-attrs/src/lib.rs), written out here so the fixture
//! carries no path dependency on the product.

/// The stub target. Contracted.
#[cfg_attr(kani, kani::ensures(|result| *result <= 10_000))]
pub fn rate(tier: u8) -> u32 {
    if tier == 0 { 150 } else { 90 }
}

/// The caller. Uncontracted, so the only thing under test here is whether the
/// stub over a contracted `rate` compiles at all.
pub fn fee(amount_cents: u32, tier: u8) -> u32 {
    let bps = rate(tier).min(10_000);
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}

#[cfg(kani)]
#[allow(dead_code)]
mod proofs {
    use super::*;

    fn stub_rate(_tier: u8) -> u32 {
        let r: u32 = kani::any();
        kani::assume(r <= 10_000);
        r
    }

    /// Should VERIFY: under the stub, `bps <= 10_000`, so the fee never
    /// exceeds the amount. The question this fixture asks is not the verdict
    /// but whether the crate compiles at all.
    #[kani::proof]
    #[kani::stub(rate, stub_rate)]
    fn check_fee_over_contracted_stub() {
        let amount_cents: u32 = kani::any();
        kani::assume(amount_cents <= 100_000_000);
        let tier: u8 = kani::any();
        assert!(fee(amount_cents, tier) <= amount_cents, "fee exceeded the amount");
    }
}
