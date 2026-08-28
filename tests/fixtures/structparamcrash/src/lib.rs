//! A crash in a function whose only parameter is a struct loses its witness
//! (docs/review-structs-enums.md's "Also fix" list, 2026-08-28): proptest's
//! shrunk minimal input for a struct parameter describes its *fields*
//! (`(start, end)` for `Window`'s two), never a single value per declared
//! parameter -- so a one-struct-parameter fn has one declared parameter and
//! two recovered values, and the plain positional recovery this crate used
//! discarded the witness the moment the counts disagreed, reporting a tool
//! error instead of the real crash. The receiver path was taught to carry
//! a count mismatch through as one opaque field in the same window this
//! shape was found in; this one was not.

pub struct Window {
    pub start: u32,
    pub end: u32,
}

/// Panics whenever `end < start` -- an ordinary unchecked subtraction bug,
/// not a contrived one. `*result >= 0` can never itself be false on a
/// `u32`, so the only way this promise breaks is by the function not
/// returning at all.
#[ply::ensures(|result| *result >= 0)]
pub fn width(r: Window) -> u32 {
    r.end - r.start
}
