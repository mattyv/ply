//! The same unclaimed callee as `tests/fixtures/unclaimedcallee`, reached the
//! way most Rust reaches a helper: through a `use` import, called by its bare
//! name. Nothing about the *claim* changes -- `tiered_fee` still promises the
//! same thing, and `legacy_rate` is still a body no contract describes -- so
//! §5.5's refusal must fire here exactly as it does there.
//!
//! Before 2026-08-25 it did not: the resolver never read `use` declarations,
//! so a bare-name call it could not find among the caller's own top-level
//! `fn` items classified `Unresolved`, and `Unresolved` meant descend. One
//! `use` line turned a loud refusal into a clean `bounded(2)` over a body
//! nobody vouched for.

/// An ordinary first-party module. No contract on anything in it.
mod rates {
    pub fn legacy_rate(tier: u8) -> u32 {
        if tier == 0 { 150 } else { 90 }
    }

    pub fn cap_bps(bps: u32) -> u32 {
        if bps > 10_000 { 10_000 } else { bps }
    }
}

// A nested group with a rename: three spellings of the same hole in one line.
use rates::{cap_bps as capped, legacy_rate};

#[ply::requires(amount_cents <= 100_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn tiered_fee(amount_cents: u32, tier: u8) -> u32 {
    let bps = capped(legacy_rate(tier));
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}
