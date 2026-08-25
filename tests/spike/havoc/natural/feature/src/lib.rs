//! `billing` — the new order-pricing feature, written beside `catalog`.
//!
//! **These six functions are the sample the whole experiment turns on.** They
//! were written from a one-line task description each, in the ordinary style
//! of the crate they would live in, before a single Kani run — not designed to
//! survive an unconstrained callee and not sabotaged to fail one. Each covers
//! one of the shapes the brief named: arithmetic trusting a returned rate; a
//! quantity times a returned price; an index derived from a returned length; a
//! division by a returned divisor; an accumulation over a returned count; a
//! subtraction against a returned limit.
//!
//! Contracts are written in 004's own idiom — a `requires` bounding the inputs
//! to the module's stated domain, an `ensures` naming what the function
//! promises. `#[cfg_attr(kani, kani::x(..))]` is written out literally: it is
//! exactly what `#[ply::x(..)]` expands to (crates/ply-attrs/src/lib.rs), so
//! this crate stands alone.

/// The largest single order this pipeline handles, in cents.
pub const MAX_ORDER_CENTS: u32 = 100_000_000;

/// N1 — arithmetic that trusts a returned rate.
///
/// What an order costs the customer: the net amount plus the VAT this
/// region charges on it.
#[cfg_attr(kani, kani::requires(net_cents <= 100_000_000))]
#[cfg_attr(kani, kani::ensures(|result| *result >= net_cents))]
pub fn gross_cents(net_cents: u32, region: u8) -> u32 {
    let vat = net_cents * catalog::vat_bps(region) / 10_000;
    net_cents + vat
}

/// N2 — a quantity multiplied by a returned price.
///
/// What one order line costs: `units` items at this band's list price.
#[cfg_attr(kani, kani::requires(units <= 1_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= 100_000_000))]
pub fn line_total_cents(units: u32, band: u8) -> u32 {
    units * catalog::unit_price_cents(band)
}

/// N3 — an index derived from a returned length.
///
/// The list price of the top band this build configures, read off the
/// customer's own negotiated rate card.
#[cfg_attr(kani, kani::requires(card[0] <= 100_000 && card[1] <= 100_000
                                && card[2] <= 100_000 && card[3] <= 100_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= 100_000))]
pub fn top_band_price_cents(card: [u32; 4]) -> u32 {
    let top = catalog::band_count() - 1;
    card[top]
}

/// N4 — a division by a returned divisor.
///
/// How many pick batches a list of `total` items will take.
#[cfg_attr(kani, kani::requires(total <= 1_000_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= total))]
pub fn batches_needed(total: u32) -> u32 {
    let size = catalog::batch_size();
    (total + size - 1) / size
}

/// N5 — an accumulation over a returned count.
///
/// What a full manifest weighs, in grams, at `unit_grams` a line.
#[cfg_attr(kani, kani::requires(unit_grams <= 10_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= 10_000_000))]
pub fn manifest_weight_grams(unit_grams: u32) -> u64 {
    let mut total: u64 = 0;
    for _ in 0..catalog::manifest_lines() {
        total += unit_grams as u64;
    }
    total
}

/// N6 — a subtraction against a returned limit.
///
/// How much of this account's spend limit is left once a charge of
/// `amount_cents` is applied.
#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000))]
#[cfg_attr(kani, kani::ensures(|result| *result <= 100_000_000))]
pub fn remaining_limit_cents(account: u64, amount_cents: u32) -> u32 {
    let limit = catalog::spend_limit_cents(account);
    if amount_cents > limit {
        0
    } else {
        limit - amount_cents
    }
}

#[cfg(kani)]
mod ply_generated;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_six_do_what_they_say_on_real_data() {
        assert_eq!(gross_cents(10_000, 0), 11_900);
        assert_eq!(line_total_cents(3, 1), 2_997);
        assert_eq!(top_band_price_cents([100, 200, 300, 400]), 400);
        assert_eq!(batches_needed(101), 3);
        assert_eq!(manifest_weight_grams(250), 6_000);
        assert_eq!(remaining_limit_cents(41, 100_000), 400_000);
    }
}
