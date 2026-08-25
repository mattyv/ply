//! Hand-written in the shape `crates/ply-core/src/harness.rs::generate_proof_module`
//! emits. The only difference from `tests/spike/havoc/`'s copy is what goes in the
//! stub body: there it was a bare `kani::any()` (the empty contract); here it is
//! `kani::any()` constrained by **one clause, written once, applied to every
//! function in the region** — PROMISES.md's B1.
//!
//! The macro is not a convenience. It is the experiment: `catalog_promise!` is
//! written in exactly one place and expanded into all six stubs, so "one line
//! instead of six" is a property of this file you can see rather than a claim.
//!
//! Three harnesses per caller:
//!   * `ply_region_<fn>`    — the region promise (B1, or B1-tight under `--features tight`)
//!   * `ply_percallee_<fn>` — PROMISES.md's per-callee clause for that one callee
//!   * `ply_base_<fn>`      — no stub at all (the havoc file's baseline, kept so a
//!                            failure can still be attributed to the stub)
#[cfg(kani)]
use super::*;

// ===========================================================================
// THE REGION PROMISE for `catalog`. Written once. PROMISES.md B1.
//
//   "Nothing in this module returns a value above 1,000,000."
// ===========================================================================
#[cfg(not(feature = "tight"))]
macro_rules! catalog_promise {
    ($result:expr) => {
        $result <= 1_000_000
    };
}

// PROMISES.md B1-tight: the tightest ceiling still true of `catalog` as written.
// Pre-registered so nobody has to wonder whether a loose ceiling was chosen.
#[cfg(feature = "tight")]
macro_rules! catalog_promise {
    ($result:expr) => {
        $result <= 500_000
    };
}

// ------------------------------------------- the region stubs: one clause, six uses
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_region_stub_vat_bps(region: u8) -> u32 {
    let result: u32 = kani::any();
    kani::assume(catalog_promise!(result));
    result
}
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_region_stub_unit_price_cents(band: u8) -> u32 {
    let result: u32 = kani::any();
    kani::assume(catalog_promise!(result));
    result
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_region_stub_band_count() -> usize {
    let result: usize = kani::any();
    kani::assume(catalog_promise!(result));
    result
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_region_stub_batch_size() -> u32 {
    let result: u32 = kani::any();
    kani::assume(catalog_promise!(result));
    result
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_region_stub_manifest_lines() -> usize {
    let result: usize = kani::any();
    kani::assume(catalog_promise!(result));
    result
}
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_region_stub_spend_limit_cents(account: u64) -> u32 {
    let result: u32 = kani::any();
    kani::assume(catalog_promise!(result));
    result
}

// -------------------------------- the per-callee stubs: six clauses, one each
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_percallee_stub_vat_bps(region: u8) -> u32 {
    let result: u32 = kani::any();
    kani::assume(result <= 10_000);
    result
}
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_percallee_stub_unit_price_cents(band: u8) -> u32 {
    let result: u32 = kani::any();
    kani::assume(result <= 100_000);
    result
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_percallee_stub_band_count() -> usize {
    let result: usize = kani::any();
    kani::assume(result >= 1 && result <= 64);
    result
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_percallee_stub_batch_size() -> u32 {
    let result: u32 = kani::any();
    kani::assume(result >= 1 && result <= 10_000);
    result
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_percallee_stub_manifest_lines() -> usize {
    let result: usize = kani::any();
    kani::assume(result >= 1 && result <= 1_000);
    result
}
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_percallee_stub_spend_limit_cents(account: u64) -> u32 {
    let result: u32 = kani::any();
    kani::assume(result <= 1_000_000);
    result
}

// ------------------------------------------------------------------------- N1
#[cfg(kani)]
#[kani::proof_for_contract(gross_cents)]
#[kani::stub(catalog::vat_bps, ply_region_stub_vat_bps)]
fn ply_region_gross_cents() {
    let net_cents: u32 = kani::any();
    let region: u8 = kani::any();
    gross_cents(net_cents, region);
}

#[cfg(kani)]
#[kani::proof_for_contract(gross_cents)]
#[kani::stub(catalog::vat_bps, ply_percallee_stub_vat_bps)]
fn ply_percallee_gross_cents() {
    let net_cents: u32 = kani::any();
    let region: u8 = kani::any();
    gross_cents(net_cents, region);
}

#[cfg(kani)]
#[kani::proof_for_contract(gross_cents)]
fn ply_base_gross_cents() {
    let net_cents: u32 = kani::any();
    let region: u8 = kani::any();
    gross_cents(net_cents, region);
}

// ------------------------------------------------------------------------- N2
#[cfg(kani)]
#[kani::proof_for_contract(line_total_cents)]
#[kani::stub(catalog::unit_price_cents, ply_region_stub_unit_price_cents)]
fn ply_region_line_total_cents() {
    let units: u32 = kani::any();
    let band: u8 = kani::any();
    line_total_cents(units, band);
}

#[cfg(kani)]
#[kani::proof_for_contract(line_total_cents)]
#[kani::stub(catalog::unit_price_cents, ply_percallee_stub_unit_price_cents)]
fn ply_percallee_line_total_cents() {
    let units: u32 = kani::any();
    let band: u8 = kani::any();
    line_total_cents(units, band);
}

// ------------------------------------------------------------------------- N3
#[cfg(kani)]
#[kani::proof_for_contract(top_band_price_cents)]
#[kani::stub(catalog::band_count, ply_region_stub_band_count)]
fn ply_region_top_band_price_cents() {
    let card: [u32; 4] = kani::any();
    top_band_price_cents(card);
}

#[cfg(kani)]
#[kani::proof_for_contract(top_band_price_cents)]
#[kani::stub(catalog::band_count, ply_percallee_stub_band_count)]
fn ply_percallee_top_band_price_cents() {
    let card: [u32; 4] = kani::any();
    top_band_price_cents(card);
}

// ------------------------------------------------------------------------- N4
#[cfg(kani)]
#[kani::proof_for_contract(batches_needed)]
#[kani::stub(catalog::batch_size, ply_region_stub_batch_size)]
fn ply_region_batches_needed() {
    let total: u32 = kani::any();
    batches_needed(total);
}

#[cfg(kani)]
#[kani::proof_for_contract(batches_needed)]
#[kani::stub(catalog::batch_size, ply_percallee_stub_batch_size)]
fn ply_percallee_batches_needed() {
    let total: u32 = kani::any();
    batches_needed(total);
}

// ------------------------------------------------------------------------- N5
#[cfg(kani)]
#[kani::proof_for_contract(manifest_weight_grams)]
#[kani::stub(catalog::manifest_lines, ply_region_stub_manifest_lines)]
fn ply_region_manifest_weight_grams() {
    let unit_grams: u32 = kani::any();
    manifest_weight_grams(unit_grams);
}

#[cfg(kani)]
#[kani::proof_for_contract(manifest_weight_grams)]
#[kani::stub(catalog::manifest_lines, ply_percallee_stub_manifest_lines)]
fn ply_percallee_manifest_weight_grams() {
    let unit_grams: u32 = kani::any();
    manifest_weight_grams(unit_grams);
}

// ------------------------------------------------------------------------- N6
#[cfg(kani)]
#[kani::proof_for_contract(remaining_limit_cents)]
#[kani::stub(catalog::spend_limit_cents, ply_region_stub_spend_limit_cents)]
fn ply_region_remaining_limit_cents() {
    let account: u64 = kani::any();
    let amount_cents: u32 = kani::any();
    remaining_limit_cents(account, amount_cents);
}

#[cfg(kani)]
#[kani::proof_for_contract(remaining_limit_cents)]
#[kani::stub(catalog::spend_limit_cents, ply_percallee_stub_spend_limit_cents)]
fn ply_percallee_remaining_limit_cents() {
    let account: u64 = kani::any();
    let amount_cents: u32 = kani::any();
    remaining_limit_cents(account, amount_cents);
}
