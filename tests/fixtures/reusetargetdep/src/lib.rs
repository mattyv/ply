//! Fixture for The-Ply-Spec.md §5.2a, third sibling to `reusehelper` and
//! `reuseexamplehelper`.
//!
//! Those two cover a helper in the crate's own source. This one covers a
//! helper in a **path dependency declared under a platform predicate**
//! (`[target.'cfg(unix)'.dependencies]`). Cargo compiles it exactly as it
//! compiles a plain `[dependencies]` entry, and the check runs straight
//! through it -- but Ply read only `[dependencies]` and `[dev-dependencies]`
//! table headers, so this crate's source was hashed nowhere at all.
//! Rewriting `platform::scale` left the fingerprint byte-identical and
//! served the old pass over code that had changed.
//!
//! Reported by external review 2026-09-06.

/// Breaks its own guarantee the moment `platform::scale` does.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result >= x)]
pub fn doubled(x: u32) -> u32 {
    platform::scale(x)
}
