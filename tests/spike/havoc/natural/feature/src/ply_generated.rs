//! Hand-written in the shape `crates/ply-core/src/harness.rs::generate_proof_module`
//! emits, with the one deliberate difference the experiment is about: each
//! `ply_stub_*` body is a bare `kani::any()` with **no `kani::assume`**. That is
//! what a `given:` region's stub would be — the empty contract.
//!
//! Two harnesses per caller:
//!   * `ply_havoc_<fn>`  — the callee replaced by an unconstrained return;
//!   * `ply_base_<fn>`   — no stub at all, the real callee body inlined.
//!
//! The baseline row is not decoration. Without it a failure cannot be
//! attributed: a caller that fails with the real callee tells you nothing about
//! havoc. Only rows whose baseline is green measure the thing being asked.
#[cfg(kani)]
use super::*;

// ------------------------------------------------------------- the havoc stubs
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_stub_vat_bps(region: u8) -> u32 {
    kani::any()
}
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_stub_unit_price_cents(band: u8) -> u32 {
    kani::any()
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_stub_band_count() -> usize {
    kani::any()
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_stub_batch_size() -> u32 {
    kani::any()
}
#[cfg(kani)]
#[allow(dead_code)]
fn ply_stub_manifest_lines() -> usize {
    kani::any()
}
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_stub_spend_limit_cents(account: u64) -> u32 {
    kani::any()
}

// ------------------------------------------------------------------------- N1
#[cfg(kani)]
#[kani::proof_for_contract(gross_cents)]
#[kani::stub(catalog::vat_bps, ply_stub_vat_bps)]
fn ply_havoc_gross_cents() {
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
#[kani::stub(catalog::unit_price_cents, ply_stub_unit_price_cents)]
fn ply_havoc_line_total_cents() {
    let units: u32 = kani::any();
    let band: u8 = kani::any();
    line_total_cents(units, band);
}

#[cfg(kani)]
#[kani::proof_for_contract(line_total_cents)]
fn ply_base_line_total_cents() {
    let units: u32 = kani::any();
    let band: u8 = kani::any();
    line_total_cents(units, band);
}

// ------------------------------------------------------------------------- N3
#[cfg(kani)]
#[kani::proof_for_contract(top_band_price_cents)]
#[kani::stub(catalog::band_count, ply_stub_band_count)]
fn ply_havoc_top_band_price_cents() {
    let card: [u32; 4] = kani::any();
    top_band_price_cents(card);
}

#[cfg(kani)]
#[kani::proof_for_contract(top_band_price_cents)]
fn ply_base_top_band_price_cents() {
    let card: [u32; 4] = kani::any();
    top_band_price_cents(card);
}

// ------------------------------------------------------------------------- N4
#[cfg(kani)]
#[kani::proof_for_contract(batches_needed)]
#[kani::stub(catalog::batch_size, ply_stub_batch_size)]
fn ply_havoc_batches_needed() {
    let total: u32 = kani::any();
    batches_needed(total);
}

#[cfg(kani)]
#[kani::proof_for_contract(batches_needed)]
fn ply_base_batches_needed() {
    let total: u32 = kani::any();
    batches_needed(total);
}

// ------------------------------------------------------------------------- N5
#[cfg(kani)]
#[kani::proof_for_contract(manifest_weight_grams)]
#[kani::stub(catalog::manifest_lines, ply_stub_manifest_lines)]
fn ply_havoc_manifest_weight_grams() {
    let unit_grams: u32 = kani::any();
    manifest_weight_grams(unit_grams);
}

#[cfg(kani)]
#[kani::proof_for_contract(manifest_weight_grams)]
fn ply_base_manifest_weight_grams() {
    let unit_grams: u32 = kani::any();
    manifest_weight_grams(unit_grams);
}

// ------------------------------------------------------------------------- N6
#[cfg(kani)]
#[kani::proof_for_contract(remaining_limit_cents)]
#[kani::stub(catalog::spend_limit_cents, ply_stub_spend_limit_cents)]
fn ply_havoc_remaining_limit_cents() {
    let account: u64 = kani::any();
    let amount_cents: u32 = kani::any();
    remaining_limit_cents(account, amount_cents);
}

#[cfg(kani)]
#[kani::proof_for_contract(remaining_limit_cents)]
fn ply_base_remaining_limit_cents() {
    let account: u64 = kani::any();
    let amount_cents: u32 = kani::any();
    remaining_limit_cents(account, amount_cents);
}
