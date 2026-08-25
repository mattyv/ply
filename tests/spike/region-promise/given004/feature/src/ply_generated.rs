//! Region A's harnesses. Same shape as `tests/spike/havoc/given004`'s copy, with
//! the stub body carrying PROMISES.md's A1 instead of a bare `kani::any()`.
//!
//! The macro is written once and expanded into every stub for the region — which
//! for `ledger::fees` today means exactly one stub. That 1:1 ratio is the finding
//! for this region, not an oversight; PROMISES.md records why `ledger` as a whole
//! admits no single clause.
#[cfg(kani)]
use super::*;

// ===========================================================================
// THE REGION PROMISE for `ledger::fees`. Written once. PROMISES.md A1.
//
//   "Every rate this schedule returns is a rate in basis points: at most
//    10,000, which is one hundred percent."
// ===========================================================================
macro_rules! fees_promise {
    ($result:expr) => {
        $result <= 10_000
    };
}

#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_region_stub_bps_for_tier(tier: u8) -> u32 {
    let result: u32 = kani::any();
    kani::assume(fees_promise!(result));
    result
}

/// The empty contract, kept for the mutation rows: this is havoc, byte-for-byte
/// what `tests/spike/havoc` ran. Rows that delete the promise stub in favour of
/// this one are checking that the promise is load-bearing, not decorative.
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_havoc_stub_bps_for_tier(tier: u8) -> u32 {
    kani::any()
}

// ------------------------------------------------------- the flagship crossing
#[cfg(kani)]
#[kani::proof_for_contract(tier_fee_cents)]
#[kani::stub(ledger::fees::bps_for_tier, ply_region_stub_bps_for_tier)]
fn ply_region_tier_fee_cents() {
    let amount_cents: u32 = kani::any();
    let tier: u8 = kani::any();
    tier_fee_cents(amount_cents, tier);
}

// ------------------------------------------------------ the transitive crossing
#[cfg(kani)]
#[kani::proof_for_contract(approve_withdrawal)]
#[kani::stub(ledger::fees::bps_for_tier, ply_region_stub_bps_for_tier)]
fn ply_region_approve_withdrawal() {
    let amount_cents: u32 = kani::any();
    let balance_cents: i64 = kani::any();
    let tier: u8 = kani::any();
    approve_withdrawal(amount_cents, balance_cents, tier);
}

// -------------------------------------------------------------- mutation rows
/// Same claim, the empty contract instead of the region promise. Used with the
/// caller's own `.min(10_000)` deleted, so the promise is the only thing that
/// could carry the proof.
#[cfg(kani)]
#[kani::proof_for_contract(tier_fee_cents)]
#[kani::stub(ledger::fees::bps_for_tier, ply_havoc_stub_bps_for_tier)]
fn ply_havoc_tier_fee_cents() {
    let amount_cents: u32 = kani::any();
    let tier: u8 = kani::any();
    tier_fee_cents(amount_cents, tier);
}
