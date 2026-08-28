//! docs/review-silent-narrowing.md finding 1, 2026-08-28, and the
//! coordinator's own follow-up review of that fix: `Till`'s only mutating
//! operations live in *other* `impl` blocks scattered across other files --
//! completely ordinary Rust, and every one of them is one of the handful of
//! ways a real crate actually writes "this impl is for the type declared
//! over there":
//!
//! - `more.rs`: `impl Till` after `use crate::till::Till;` -- the plain,
//!   bare spelling (already fixed once, kept here so it can never silently
//!   regress out from under the others).
//! - `till/super_ops.rs`: `impl super::Till` -- the parent-module keyword,
//!   exactly what a submodule of the type's own module writes, and exactly
//!   the spelling the first fix's own test could not surface (its own
//!   fixture only ever used the bare form).
//! - `crate_ops.rs`: `impl crate::till::Till` -- the crate-root keyword.
//! - `till.rs` itself: a *second* `impl self::Till` block, alongside the
//!   checked method's own -- the same-module keyword.
//! - `alias_ops.rs`: `use crate::till::Till as T; impl T` -- reached only
//!   through a local rename, never `Till`'s own name at all.
//!
//! Every one of these is a real, ordinary way to split a type's `impl`
//! blocks across a crate, and every one of them changes `Till`'s state.
//! `Till::total`'s promise (always 0) is false the moment any one of them
//! runs:
//!
//! ```ignore
//! let mut t = Till::new();
//! t.bare_bump(1);   // or super_bump, crate_bump, self_bump, alias_bump
//! t.total() // -> nonzero, not 0
//! ```
//!
//! All five are covered by *one* fixture and *one* test (the coordinator's
//! own instruction: spellings that resolve to the same type must be
//! checked together, not as separate fixtures that could drift apart) --
//! the acceptance bar is that every one of the five is confirmed into the
//! same operation pool, and the run reports a genuine violation, not a
//! partial disclosure.

pub mod alias_ops;
pub mod crate_ops;
pub mod more;
pub mod till;
