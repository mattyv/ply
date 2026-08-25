//! Hand-written in the shape `crates/ply-core/src/harness.rs::generate_proof_module`
//! emits -- `use super::*`, a `ply_stub_*` fn, the `#[kani::proof_for_contract]` +
//! `#[kani::stub]` pair -- with **one deliberate difference**: the stub body has no
//! `kani::assume`. That is the whole experiment. A `given:` region declares no
//! contract, so the stub it would generate is the empty one: an unconstrained
//! `kani::any()` of the callee's return type, and nothing else.
//!
//! Nothing here implements `given:`. This is the hypothesis, hand-wired, so the
//! feature does not have to be built to find out whether it would pay.
#[cfg(kani)]
use super::*;

// -------------------------------------------------------------- the havoc stub
// `ledger::fees::bps_for_tier(tier: u8) -> u32`, replaced by "some u32".
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_stub_bps_for_tier(tier: u8) -> u32 {
    kani::any()
}

// ------------------------------------------------- row: tier_fee_cents / havoc
// 004's flagship crossing. The falsifiable prediction on record is that this
// passes, because of `tier_fee_cents`'s own `.min(10_000)`.
#[cfg(kani)]
#[kani::proof_for_contract(tier_fee_cents)]
#[kani::stub(ledger::fees::bps_for_tier, ply_stub_bps_for_tier)]
fn ply_proof_tier_fee_cents_havoc() {
    let amount_cents: u32 = kani::any();
    let tier: u8 = kani::any();
    tier_fee_cents(amount_cents, tier);
}

// ---------------------------------------------- row: tier_fee_cents / baseline
// The same claim with no stub at all: Kani descends into the real
// `BTreeMap`-behind-`OnceLock` lookup. 004 s3 measured this as `timeout` at both
// 120s and 600s; re-run here at the same 300s cap as every havoc row so the two
// numbers in this file are comparable.
#[cfg(kani)]
#[kani::proof_for_contract(tier_fee_cents)]
fn ply_proof_tier_fee_cents_baseline() {
    let amount_cents: u32 = kani::any();
    let tier: u8 = kani::any();
    tier_fee_cents(amount_cents, tier);
}

// -------------------------------------------- row: approve_withdrawal / havoc
// The transitive crossing: `approve_withdrawal` never names `ledger`, it calls
// `tier_fee_cents`, which does. A region declaration covers call sites nobody
// enumerated, so this is the shape the region claims to buy over the per-callee
// fallback. 004 claims this fn at `fuzz(256), test`, not `bounded` -- the
// `bounded` claim here is this spike's, and is marked as such in FINDINGS.md.
#[cfg(kani)]
#[kani::proof_for_contract(approve_withdrawal)]
#[kani::stub(ledger::fees::bps_for_tier, ply_stub_bps_for_tier)]
fn ply_proof_approve_withdrawal_havoc() {
    let amount_cents: u32 = kani::any();
    let balance_cents: i64 = kani::any();
    let tier: u8 = kani::any();
    approve_withdrawal(amount_cents, balance_cents, tier);
}

// ------------------------------- row: tier_fee_cents / declared boundary contract
// The cost baseline the proposal is measured against: §5.5's second branch, the
// route a `given:` region is meant to relieve. Identical to the havoc harness
// except for the one `kani::assume` -- which is exactly what a hand-written
// per-callee contract buys. 004 s5 measured this path end-to-end through the
// product at 201.77s; this harness measures the same shape on this machine so
// the two numbers in this file are comparable.
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_stub_bps_for_tier_contract(tier: u8) -> u32 {
    let __ply_result: u32 = kani::any();
    kani::assume((|result: &u32| *result <= 10_000)(&__ply_result));
    __ply_result
}

#[cfg(kani)]
#[kani::proof_for_contract(tier_fee_cents)]
#[kani::stub(ledger::fees::bps_for_tier, ply_stub_bps_for_tier_contract)]
fn ply_proof_tier_fee_cents_contract() {
    let amount_cents: u32 = kani::any();
    let tier: u8 = kani::any();
    tier_fee_cents(amount_cents, tier);
}

// ------------------------------------------------------ row: withdraw / refusal
// §4's third precedence row, written out so the refusal is an observed compile
// error rather than an assertion. `withdraw` takes `&mut ledger::Ledger` and
// calls `Ledger::post`, which takes `&mut self`. There is no harness to write:
// Kani cannot build a `Ledger` (private `BTreeMap`/`Vec` fields, no `Arbitrary`),
// and a havoc stub of `post` would have to invent a return value *and* silently
// assume the receiver was left alone -- the fail-open pattern §4 refuses by name.
// Gated behind a feature so it does not break the crate for the other rows.
#[cfg(all(kani, feature = "withdraw_row"))]
#[kani::proof_for_contract(withdraw)]
fn ply_havoc_withdraw() {
    let mut accounts: ledger::Ledger = kani::any();
    let account: ledger::AccountId = kani::any();
    let amount_cents: u32 = kani::any();
    let tier: u8 = kani::any();
    withdraw(&mut accounts, account, amount_cents, tier);
}
