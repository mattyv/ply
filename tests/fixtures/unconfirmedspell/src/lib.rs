//! The safety net the qualified-spelling resolution needs (coordinator
//! review of docs/review-silent-narrowing.md's own fix, 2026-08-28): "if
//! any of those cannot be resolved confidently, they must land in the
//! excluded-operations list by name with a true reason -- never silently
//! absent while the run claims completeness."
//!
//! Two structs both bare-named `Till` exist in this crate -- `till::Till`
//! (the real target) and this crate root's own, unrelated `Till`. Because
//! the bare name `Till` is ambiguous crate-wide, Ply cannot confirm which
//! `Till` a qualified spelling like `super::Till` refers to (even though,
//! read as ordinary Rust, `till::super_ops`'s own `super::Till` obviously
//! means `till::Till` -- an unrelated same-named type elsewhere in the
//! crate is exactly the ambiguity Ply's own struct/enum work already
//! refuses to guess through). So the mutator this impl block adds must be
//! named as an exclusion, not silently pooled and not silently dropped --
//! `till::Till::total`'s promise stays unqualified `fuzzed(n)` (Ply
//! genuinely cannot call this operation), but the run must say so.
pub mod till;

pub struct Till {
    pub irrelevant: u32,
}
