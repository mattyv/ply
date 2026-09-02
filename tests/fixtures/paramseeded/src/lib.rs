//! Composition (TODO.md, "make the sampling engine's decision recursive"),
//! 2026-09-02: `width`'s own parameter is `Option<String>` -- a `String`
//! nested inside another type. Before this task, `RustType` deliberately
//! never built one nested this way, so Ply refused `width` outright with
//! "parameter(s) label: Option<String> use a type neither the bounded
//! (Kani) nor the fuzz (proptest) codegen builds inputs for", full stop --
//! this fixture used to need a corpus seeded from an `examples:` entry to
//! work around that (`docs/reach-measurement-2.md`'s own gap). Composition
//! closes the gap directly: a `String` is buildable alone, so nesting it
//! inside `Option` no longer refuses it, and no seed is needed at all.

#[ply::ensures(|result| *result >= 0)]
pub fn width(label: Option<String>) -> usize {
    label.map(|s| s.len()).unwrap_or(0)
}
