//! Acceptance fixture for the sampling/proving split's second headline case
//! (task, 2026-08-27): `String` is fuzz-supported, never bounded-supported,
//! refused *by name* (not silently downgraded) when `bounded` is asked for
//! anyway -- the same asymmetry `floatsample` already exercises for `f32`/
//! `f64`.
//!
//! `preview` is the decisive case: a genuinely broken string function, with
//! a **false** promise (`result.chars().count() <= 3`), caught by sampling.
//! It naively truncates by *byte* count (`String::truncate`) rather than by
//! *character* count, which panics the moment the third byte falls inside a
//! multi-byte character -- the exact byte-vs-character confusion this type
//! exists to catch (see `RustType::String`'s own doc in harness.rs: "the
//! richest bug territory"). Ply's default `String` sampling sometimes
//! generates genuine multi-byte content (accented letters, CJK, emoji), so
//! this is caught, not merely theoretical.

#[ply::ensures(|result| *result == old(s).len())]
pub fn byte_len(s: String) -> usize {
    s.len()
}

// The decisive, genuinely broken case (false promise, caught by sampling):
// truncates by *byte* count instead of *character* count, so it panics on
// multi-byte input whose third byte falls mid-character.
#[ply::ensures(|result| result.chars().count() <= 3)]
pub fn preview(s: String) -> String {
    let mut out = s;
    if out.len() > 3 {
        out.truncate(3);
    }
    out
}

#[ply::ensures(|result| *result == old(s).len())]
pub fn byte_len_bounded(s: String) -> usize {
    s.len()
}

// A second, non-panicking broken case: promises to count *characters* but
// actually returns the *byte* length -- wrong for any multi-byte input,
// without ever panicking. Exercises the ordinary (non-panic) postcondition-
// failure path, where the reported string comes back through Ply's own
// marker decoding rather than proptest's panic-shrink report.
#[ply::ensures(|result| *result == old(s).chars().count())]
pub fn char_count_wrong(s: String) -> usize {
    s.len()
}
