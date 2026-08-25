//! `withdrawal` — the new feature: what a withdrawal costs, and whether the
//! account can afford it.
//!
//! This is the "written inside the fragment from line one" half of vetting
//! scenario 004. Every function here is a top-level free function over
//! scalars, with a `#[ply::requires]`/`#[ply::ensures]` contract, because
//! that is what The-Ply-Spec.md §5.4b's measured fragment — and, more
//! narrowly, what `cargo ply verify` actually implements today — can check.
//! `tier_fee_cents` is the boundary: fragment-clean signature, body that
//! calls two-year-old code Ply has never seen.
//!
//! Transcribed for the havoc spike, two changes and no others:
//! `#[ply::x(..)]` is written out as the `#[cfg_attr(kani, kani::x(..))]`
//! it expands to (crates/ply-attrs/src/lib.rs), so this crate needs no
//! path dependency on the product; and 004's own s1->s2 one-line overflow
//! fix (widen the fee product to u64) is applied, because every stage of
//! 004 after s1 runs with it applied.

/// Money never exceeds ten million euro in one movement; the payment rails
/// reject anything larger long before this code sees it.
pub const MAX_MOVEMENT_CENTS: u32 = 100_000_000;

/// One hundred percent, in basis points.
pub const FULL_RATE_BPS: u32 = 10_000;

/// The fee, in cents, charged on a withdrawal of `amount_cents` at a rate of
/// `bps` basis points. Rounds down, in the customer's favour.
#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000 && bps <= 10_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= amount_cents))]
pub fn fee_cents(amount_cents: u32, bps: u32) -> u32 {
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}

/// What actually leaves the account: the withdrawal plus its fee.
#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000 && bps <= 10_000))]
#[cfg_attr(kani, kani::ensures(|result| *result >= amount_cents))]
pub fn total_debit_cents(amount_cents: u32, bps: u32) -> u32 {
    amount_cents + fee_cents(amount_cents, bps)
}

/// The fee for one withdrawal at this account's tier.
///
/// **The boundary.** The rate comes from `ledger::fees`, the existing
/// table-driven fee schedule — unannotated, unclaimed, `BTreeMap`-backed
/// code that predates this feature by two years. The `.min` is the ordinary
/// defensive clamp any caller of a table lookup writes: the table is data,
/// and data can be wrong.
#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= amount_cents))]
pub fn tier_fee_cents(amount_cents: u32, tier: u8) -> u32 {
    let bps = ledger::fees::bps_for_tier(tier).min(10_000);
    fee_cents(amount_cents, bps)
}

/// Whether a withdrawal of `amount_cents` clears against a balance of
/// `balance_cents`, once this tier's fee is added.
#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000))]
#[cfg_attr(kani, kani::ensures(|result| !*result || balance_cents > 0))]
pub fn approve_withdrawal(amount_cents: u32, balance_cents: i64, tier: u8) -> bool {
    if amount_cents == 0 || balance_cents <= 0 {
        return false;
    }
    let debit = amount_cents as i64 + tier_fee_cents(amount_cents, tier) as i64;
    debit <= balance_cents
}

/// Performs a withdrawal against the existing ledger: checks that it clears,
/// then posts the movement and its fee.
///
/// **The shell.** This is where the feature actually meets `ledger`, and
/// nothing about this signature is inside the fragment: the first parameter
/// is a legacy struct with private fields and a `BTreeMap` inside it, and
/// `AccountId` is the module's own alias for `u64`. What this function owes
/// its caller is a two-state promise — on `true`, the account's balance fell
/// by exactly the amount plus the fee — which needs `old()` over
/// `Ledger::balance`, a method call on a struct receiver; §5.4a's expression
/// subset admits neither, so the contract below is the weak one that fits.
#[cfg_attr(kani, kani::requires(amount_cents > 0 && amount_cents <= 100_000_000))]
#[cfg_attr(kani, kani::ensures(|result| !*result || amount_cents > 0))]
pub fn withdraw(
    accounts: &mut ledger::Ledger,
    account: ledger::AccountId,
    amount_cents: u32,
    tier: u8,
) -> bool {
    if !approve_withdrawal(amount_cents, accounts.balance(account), tier) {
        return false;
    }
    let fee = tier_fee_cents(amount_cents, tier);
    accounts.post(
        account,
        -(amount_cents as i64),
        ledger::EntryKind::Withdrawal,
    );
    if fee > 0 {
        accounts.post(account, -(fee as i64), ledger::EntryKind::Fee);
    }
    true
}

#[cfg(kani)]
mod ply_generated;
