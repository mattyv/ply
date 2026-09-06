//! Fixture for The-Ply-Spec.md §5.2a, fourth sibling to `reusehelper`,
//! `reuseexamplehelper` and `reusetargetdep`.
//!
//! Those cover a helper the function calls, one only a worked example calls,
//! and one in a platform-gated dependency. This one covers a helper named
//! only in a contract **written in `ply.yaml`** rather than as a Rust
//! attribute.
//!
//! The walk reads contract expressions off the function item's own
//! attributes, which is where an inline `#[ply::ensures(..)]` lives. A
//! contract declared in the document is merged in later and never appears
//! there, so `expected` below was hashed nowhere: rewriting it changed what
//! every generated case asserts while the fingerprint stayed identical.
//!
//! Reported by external review, 2026-09-06.

/// The value the document's postcondition compares against. Nothing calls it
/// but that postcondition.
pub fn expected() -> u32 {
    7
}

/// Carries no Rust contract attribute at all: its promise is written in
/// `ply.yaml`, which is the whole point.
///
/// It takes a parameter deliberately. A zero-argument function earns
/// `tested` rather than `fuzzed(n)`, which does not match a declared
/// `fuzz(64)` -- and Ply's own guard against a doctored record then discards
/// the stored result on every run, so the claim re-runs whatever the
/// fingerprint says. The first draft of this fixture did exactly that and
/// its reuse test passed without the fix, which is the failure this file is
/// about, one level up.
pub fn answer(x: u32) -> u32 {
    x
}

/// The control: no contract of its own beyond one that names nothing, so
/// rewriting `expected` must cost it nothing.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result > x)]
pub fn bumped(x: u32) -> u32 {
    x + 1
}
