//! N1 acceptance fixture (docs/review-caveats.md): the crate under test is
//! a *member* of a bigger, ordinary multi-crate workspace (`../Cargo.toml`
//! lists `alpha` and `beta`), not its own workspace root -- the second
//! shape the review found `cargo ply verify` could not run on at all, and
//! the only workaround (giving this crate its own `[workspace]` table)
//! broke `cargo build` for the whole `wsmember` workspace.
//!
//! Same seeded bug as the `plain`/`fuzzbug` fixtures, so the acceptance
//! test can assert Ply still catches a real broken promise here too.
//!
//! Pristine "before" state: `cargo-ply verify` generates everything else
//! at run time.

#[ply::ensures(|result| *result == x)]
pub fn seeded_bug(x: u32) -> u32 {
    if x == 7 { x + 1 } else { x }
}
