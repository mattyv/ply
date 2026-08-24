//! `ledger` — account balances and the entry journal.
//!
//! This is the "already existed for two years" half of vetting scenario 004.
//! Nothing here knows Ply exists: no `ply::` attributes, no contracts, no
//! ply.yaml claim on any function in this file. It is written the way this
//! code would ordinarily be written — a `BTreeMap` of balances, a
//! `Vec`-returning query, a generic fold helper used by the reporting code,
//! private fields with an invariant (`journal` and `balances` agree) that
//! only `post` maintains.
//!
//! It is deliberately *not* a strawman: none of these choices is unusual or
//! gratuitously hostile to a verifier. They are simply the choices a Rust
//! programmer makes when nobody is thinking about proof engines.

use std::collections::BTreeMap;

pub type AccountId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Deposit,
    Withdrawal,
    Fee,
    Adjustment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub seq: u64,
    pub account: AccountId,
    pub amount_cents: i64,
    pub kind: EntryKind,
}

/// The journal plus the balances it implies. Fields are private because the
/// two must move together.
#[derive(Debug, Default)]
pub struct Ledger {
    balances: BTreeMap<AccountId, i64>,
    journal: Vec<Entry>,
    next_seq: u64,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one entry and moves the account's balance by the same amount.
    /// Returns the new entry's sequence number.
    pub fn post(&mut self, account: AccountId, amount_cents: i64, kind: EntryKind) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        *self.balances.entry(account).or_insert(0) += amount_cents;
        self.journal.push(Entry {
            seq,
            account,
            amount_cents,
            kind,
        });
        seq
    }

    /// Current balance in cents. An account nobody has posted to reads zero.
    pub fn balance(&self, account: AccountId) -> i64 {
        self.balances.get(&account).copied().unwrap_or(0)
    }

    /// Every journal entry for one account, oldest first.
    pub fn entries_for(&self, account: AccountId) -> Vec<Entry> {
        self.journal
            .iter()
            .copied()
            .filter(|e| e.account == account)
            .collect()
    }

    /// Every account the journal has ever touched.
    pub fn accounts(&self) -> Vec<AccountId> {
        self.balances.keys().copied().collect()
    }
}

/// Sums whatever `amount` pulls out of each item. The reporting code calls
/// this over entries, over statement lines, and over settlement batches.
pub fn total_by<T, F>(items: &[T], amount: F) -> i64
where
    F: Fn(&T) -> i64,
{
    items.iter().map(amount).sum()
}

/// The withdrawal fee schedule. Table-driven since the pricing rework: the
/// rates are data, looked up by account tier, with the standard rate as the
/// fallback for a tier this build does not know about.
pub mod fees {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    /// What an unrecognised tier is charged.
    pub const STANDARD_BPS: u32 = 150;

    fn schedule() -> &'static BTreeMap<u8, u32> {
        static SCHEDULE: OnceLock<BTreeMap<u8, u32>> = OnceLock::new();
        SCHEDULE.get_or_init(|| {
            let mut m = BTreeMap::new();
            m.insert(0, STANDARD_BPS); // retail
            m.insert(1, 90); // plus
            m.insert(2, 45); // business
            m.insert(3, 0); // internal transfers
            m
        })
    }

    /// Withdrawal fee rate, in basis points, for an account of `tier`.
    pub fn bps_for_tier(tier: u8) -> u32 {
        schedule().get(&tier).copied().unwrap_or(STANDARD_BPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_follows_the_journal() {
        let mut l = Ledger::new();
        l.post(7, 5_000, EntryKind::Deposit);
        l.post(7, -1_200, EntryKind::Withdrawal);
        assert_eq!(l.balance(7), 3_800);
        assert_eq!(l.entries_for(7).len(), 2);
        assert_eq!(l.balance(8), 0);
    }

    #[test]
    fn total_by_folds_entries() {
        let mut l = Ledger::new();
        l.post(1, 100, EntryKind::Deposit);
        l.post(1, 250, EntryKind::Deposit);
        assert_eq!(total_by(&l.entries_for(1), |e| e.amount_cents), 350);
    }

    #[test]
    fn unknown_tier_pays_the_standard_rate() {
        assert_eq!(fees::bps_for_tier(2), 45);
        assert_eq!(fees::bps_for_tier(99), fees::STANDARD_BPS);
    }
}
