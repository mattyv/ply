//! N1 regression fixture (docs/review-caveats.md): unlike `plain` and
//! `wsmember/alpha`, this crate already declares its own `[workspace]`
//! table, so `harness_crate` must take the *original* path unchanged --
//! register the generated harness as a member of this crate's own
//! workspace (`ensure_workspace_member`), never the new isolated one. Same
//! seeded bug as the other two, so all three fixtures earn the identical
//! verdict and only the mechanism underneath differs.
//!
//! Pristine "before" state: `cargo-ply verify` generates everything else
//! at run time.

#[ply::ensures(|result| *result == x)]
pub fn seeded_bug(x: u32) -> u32 {
    if x == 7 { x + 1 } else { x }
}
