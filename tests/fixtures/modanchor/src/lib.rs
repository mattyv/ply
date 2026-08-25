//! Where legacy code actually lives: another file.
//!
//! `boundarycontract` declares a promise for a callee sitting at the top
//! level of `src/lib.rs` -- the one place a real two-year-old function
//! almost never is. Here the same callee lives in `src/rates.rs`, reached
//! by `mod rates;` and called through an ordinary `use`. Nothing about the
//! promise changes: `rates::legacy_rate` still has no inline contract, and
//! ply.yaml still declares one for it.
//!
//! Before 2026-08-25 the claim could not be written at all. Call
//! classification followed `use` imports and file modules; anchor
//! resolution read one file and its top-level items only, so the same
//! `rates::legacy_rate` that `verify` named as unclaimed was rejected with
//! `E0301` the moment anyone tried to vouch for it.

mod rates;

use rates::legacy_rate;

/// The caller. `boundarycontract`'s body, deliberately: the same defensive
/// `.min`, the same widened product, so the two fixtures differ in exactly
/// one thing -- where the callee lives.
#[ply::requires(amount_cents <= 100_000_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn tiered_fee(amount_cents: u32, tier: u8) -> u32 {
    let bps = legacy_rate(tier).min(10_000);
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}
