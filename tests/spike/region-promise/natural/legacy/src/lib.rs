//! `catalog` — the price tables and operational settings the order pipeline
//! has read for years.
//!
//! The callee side of the naturally-written sample. Nothing here knows Ply
//! exists. Bodies are deliberately **cheap** — `match` arms and constants,
//! not `BTreeMap`-behind-`OnceLock` — for one methodological reason stated in
//! FINDINGS.md: every caller in this fixture is also run with **no stub at
//! all**, and that baseline has to be measurable. 004's `ledger` carries the
//! expensive-callee case, and is measured separately.

/// VAT rate in basis points for a sales region. Regions this build does not
/// know about are charged the standard rate.
pub fn vat_bps(region: u8) -> u32 {
    match region {
        0 => 1900,
        1 => 2100,
        2 => 700,
        3 => 0,
        _ => 1900,
    }
}

/// The list price of one unit, in cents, in a given price band.
pub fn unit_price_cents(band: u8) -> u32 {
    match band {
        0 => 1299,
        1 => 999,
        2 => 749,
        _ => 1499,
    }
}

/// How many price bands this build has configured.
pub fn band_count() -> usize {
    4
}

/// How many items the warehouse puts in one pick batch.
pub fn batch_size() -> u32 {
    50
}

/// How many lines one printed manifest holds.
pub fn manifest_lines() -> usize {
    24
}

/// The account's configured spend limit, in cents. A brand-new account has
/// no limit set yet and may not spend.
pub fn spend_limit_cents(account: u64) -> u32 {
    if account == 0 {
        0
    } else {
        500_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_region_pays_the_standard_rate() {
        assert_eq!(vat_bps(1), 2100);
        assert_eq!(vat_bps(200), 1900);
    }

    #[test]
    fn a_new_account_has_no_limit() {
        assert_eq!(spend_limit_cents(0), 0);
        assert_eq!(spend_limit_cents(41), 500_000);
    }
}
