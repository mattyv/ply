//! The acceptance question: Ply's real §5.5 boundary shape, end to end.
//!
//! A byte-for-byte copy of `tests/fixtures/boundarycontract`'s source, minus
//! the `ply-attrs` path dependency: `#[cfg_attr(kani, kani::requires(..))]` is
//! literally what `#[ply::requires(..)]` expands to
//! (crates/ply-attrs/src/lib.rs), so writing it out changes nothing that
//! reaches Kani and keeps this crate standalone.
//!
//! The generated harnesses live in `ply_generated.rs`, written in the exact
//! shape `crates/ply-core/src/harness.rs::generate_proof_module` emits.

/// The boundary callee: no `ply::` attributes, exactly like two-year-old code.
/// Its contract lives in ply.yaml -- here, in the generated stub.
pub fn legacy_rate(tier: u8) -> u32 {
    if tier == 0 { 150 } else { 90 }
}

/// The caller, carrying vetting 004's `tier_fee_cents` body shape: the
/// defensive `.min`, the widened product, and the full movement ceiling.
/// This body is why the stubbed proof is expensive.
#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= amount_cents))]
pub fn tiered_fee(amount_cents: u32, tier: u8) -> u32 {
    let bps = legacy_rate(tier).min(10_000);
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}

/// The same boundary, with a postcondition the *real* body satisfies (150 bps
/// is 1.5%, nowhere near half) but the *declared* contract does not support:
/// `ensures |result| *result <= 10_000` permits a full-rate stub, which makes
/// the fee equal the amount. A violation that exists only because a boundary
/// contract was declared too weak -- which is the failure §5.5 has to be able
/// to report, and §8 says it cannot report without a witness.
#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= amount_cents / 2))]
pub fn tiered_fee_halfclaim(amount_cents: u32, tier: u8) -> u32 {
    let bps = legacy_rate(tier).min(10_000);
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}

#[cfg(kani)]
mod ply_generated;

/// The last step of "is the witness usable?", and the one that does not need
/// Kani at all.
///
/// Kani's witness for `ply_proof_tiered_fee_halfclaim` is a triple: the two
/// harness inputs and the value the *stub* invented for `legacy_rate`. Ply's
/// D7 artifact is a rendered `#[test]` over the real code, so it can only
/// carry the first two. This is that test, written by hand at the witness
/// inputs -- and it **passes**, because the real `legacy_rate` returns 90,
/// not the 9217 the stub was free to pick. Nothing is wrong with the code;
/// the violation lives entirely in the gap between the declared contract and
/// the real body, so no test of the real body can reproduce it.
#[cfg(test)]
mod witness_replay {
    #[test]
    fn witness_does_not_reproduce_against_the_real_callee() {
        // The two harness inputs from the counterexample Kani printed
        // (`main` run, 2026-08-25). The third recorded value, 9217, is the
        // stub's return -- it has no place to go in a test of real code.
        let amount_cents: u32 = 39_663_841;
        let tier: u8 = 255;
        let result = super::tiered_fee_halfclaim(amount_cents, tier);
        assert_eq!(result, 356_974, "the real legacy_rate charges 90 bps");
        assert!(
            result <= amount_cents / 2,
            "the postcondition Kani reported violated in fact holds here"
        );
    }
}
