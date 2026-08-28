//! Narrowing fixture (docs/review-caveats.md N1): `mutate` genuinely
//! requires the generated harness to share one Cargo workspace with the
//! crate under test (`engines::mutants`' own module doc), and this crate
//! has no `[workspace]` table of its own for Ply to register it into. Ply
//! no longer adds one uninvited (see `plain`/`wsmember`), so `mutate` must
//! be refused by name here -- honestly, before cargo-mutants is ever
//! spawned -- while `fuzz` on the same function still runs and passes.
//!
//! `strong_target`'s contract is real (mirrors the `strongspec` fixture),
//! so nothing here is a false clean: `fuzz` genuinely holds, and `mutate`
//! genuinely produces no evidence for a documented, structural reason.

#[ply::requires(x < 1000 && y < 1000)]
#[ply::ensures(|result| *result == x + y)]
pub fn strong_target(x: u32, y: u32) -> u32 {
    x + y
}
