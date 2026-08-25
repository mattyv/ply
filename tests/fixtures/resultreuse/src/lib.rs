//! Fixture for The-Ply-Spec.md §5.2a: a result is recorded beside a hash of
//! everything it depended on, and re-earned the moment any of that moves.
//!
//! Two claims, because the record has to be able to reuse one and re-run the
//! other in the same run:
//!
//! - `safe_increment` stands on its own body alone. Editing it must re-run
//!   it and nothing else.
//! - `total` crosses into `legacy_rate`, which carries no contract of its
//!   own: ply.yaml declares one, so the proof stands on the promise rather
//!   than the body (§5.5). Editing that promise must re-run `total`, whose
//!   own source did not change at all.

/// The boundary callee: no `ply::` attributes, exactly like old code. Its
/// promise lives in ply.yaml.
pub fn legacy_rate(tier: u8) -> u32 {
    if tier == 0 { 150 } else { 90 }
}

/// Stands on its own body. Nothing it calls, nothing assumed.
#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn safe_increment(x: u32) -> u32 {
    x + 1
}

/// Stands on the promise declared for `legacy_rate`, not on its body.
#[ply::requires(amount <= 1_000)]
#[ply::ensures(|result| *result <= 11_000)]
pub fn total(amount: u32, tier: u8) -> u32 {
    amount + legacy_rate(tier)
}
