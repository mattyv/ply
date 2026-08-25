//! §5.4b's *preferred* bounded shape, end to end: "building a fixed array
//! and taking a symbolically-bounded subslice is how the professionals
//! express variable length". Vetting 004's finding 5 is what this fixture
//! closes -- the fragment-first way to keep a legacy lookup table out of a
//! proof is to pass the rate card in as data, and until 2026-08-25 that
//! came back `Unsupported("[u32 ; 4]")`.
//!
//! `Bps` is here because an alias is one line of ordinary Rust that used to
//! move a function out of the checkable set all by itself.

pub type Bps = u32;

#[ply::requires(amount_cents <= 100_000_000 && tier < 4)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn carded_fee_cents(amount_cents: u32, tier: u8, card_bps: [Bps; 4]) -> u32 {
    let bps = card_bps[(tier % 4) as usize];
    let bps = if bps > 10_000 { 10_000 } else { bps };
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}
