//! Two promises that say nothing, and the two different ways they lie.
//!
//! Under the design this tool is built for, a language model writes a
//! promise for each piece of old code the new code calls. A promise that is
//! empty, tautological or self-contradictory is therefore not a hypothetical
//! -- it is the realistic failure, and it is the one way per-function
//! promises can quietly lie.
//!
//! `vacuous_fee` is the killer. Its own postcondition (`*result == 0`) is
//! plainly false -- at 100_000 cents and tier 0 it returns 1500 -- and the
//! promise declared for the callee it crosses is satisfiable by no value at
//! all. An unsatisfiable assumption makes the proof hold *vacuously*, so
//! before 2026-08-25 Ply reported `bounded(2)` for it: a false claim,
//! reported green, with the impossible promise listed beside it as though it
//! were carrying weight.
//!
//! `havoc_fee`'s promise is the other failure: `|result| *result >= 0` is
//! true of every `u32`. It constrains nothing, so the callee was in effect
//! replaced by an arbitrary value -- and the run still reported the clause
//! as an assumption owed evidence, sending a reader off to discharge a debt
//! that does not exist.

/// The callee `vacuous_fee` crosses into. No inline contract; ply.yaml
/// declares an impossible one for it.
pub fn legacy_rate(tier: u8) -> u32 {
    if tier == 0 { 150 } else { 90 }
}

/// The callee `havoc_fee` crosses into. ply.yaml declares a trivially true
/// promise for it.
pub fn legacy_cap(bps: u32) -> u32 {
    if bps > 10_000 { 10_000 } else { bps }
}

/// Deliberately, provably wrong: this returns 1500 for (100_000, 0).
#[ply::requires(amount_cents <= 100_000)]
#[ply::ensures(|result| *result == 0)]
pub fn vacuous_fee(amount_cents: u32, tier: u8) -> u32 {
    let bps = legacy_rate(tier).min(10_000);
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}

/// Correct, and provable without assuming anything about `legacy_cap`.
#[ply::requires(amount_cents <= 100_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn havoc_fee(amount_cents: u32, bps_in: u32) -> u32 {
    let bps = legacy_cap(bps_in).min(10_000);
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}
