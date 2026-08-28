//! N1 acceptance fixture (docs/review-caveats.md): the exact layout `cargo
//! new --lib` produces -- no `[workspace]` table anywhere in this crate's
//! own `Cargo.toml` -- plus a genuinely seeded bug, same shape as the
//! `fuzzbug` fixture, so this fixture proves both halves at once: `cargo
//! ply verify` runs at all on an ordinary crate (it used to bail out with a
//! raw error and a stack trace before checking anything), and it still
//! catches a real broken promise once it does.
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures:
//! `cargo-ply verify` generates everything else at run time.

#[ply::ensures(|result| *result == x)]
pub fn seeded_bug(x: u32) -> u32 {
    if x == 7 { x + 1 } else { x }
}
