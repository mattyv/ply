//! Fixture for The-Ply-Spec.md §5.2a, the case its sibling `reusehelper`
//! does not reach.
//!
//! `reusehelper` covers a helper the **claimed function** calls. This one
//! covers a helper only the **worked example** calls -- code that runs on
//! every `test` check and that nothing in the function's body or its
//! contract ever mentions. The walk that decides what a result stood on
//! started at the claimed function, so `expected` below was hashed nowhere:
//! rewriting it changed what the assertion demanded while the fingerprint
//! stayed byte-identical, and the next run served the old pass.
//!
//! Reported by external review 2026-09-06, who asked for exactly this test
//! -- fresh versus cached -- rather than a unit test of the walk, because a
//! walk that returns the right set and a record that is actually invalidated
//! are two different claims.
//!
//! Two independent claims, so re-running one is visibly not re-running the
//! other.

/// The answer the example asserts against. Nothing calls it but the example.
pub fn expected() -> u32 {
    6
}

/// Its guarantee is true whatever `expected` says -- so if the run goes red
/// after `expected` is rewritten, that is the *example* failing, which is
/// the point: the example is the assertion the author wrote by hand.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result >= x)]
pub fn tripled(x: u32) -> u32 {
    x * 3
}

/// The control: `expected` is not reachable from here either, and this
/// claim has no example at all, so rewriting `expected` must cost it
/// nothing.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result > x)]
pub fn bumped(x: u32) -> u32 {
    x + 1
}
