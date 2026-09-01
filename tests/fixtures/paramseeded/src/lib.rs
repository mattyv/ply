//! TODO.md, "an example does not unblock a parameter Ply cannot build":
//! `width`'s own parameter is `Option<String>` -- a `String` nested inside
//! another type, which `RustType` deliberately never builds (see its own
//! doc) -- so before this fixture existed, Ply refused `width` outright with
//! "parameter(s) label: Option<String> use a type neither the bounded
//! (Kani) nor the fuzz (proptest) codegen builds inputs for", full stop,
//! even with the one `examples:` entry below sitting right next to it.
//!
//! The text inside an `Option<String>` is exactly as mutable as a bare
//! `String`'s, so one `examples:` entry is enough to seed a real corpus and
//! mutate it, the same way a receiver constructor's own gated text
//! parameter already does -- reusing that exact apparatus rather than a
//! second one.

#[ply::ensures(|result| *result >= 0)]
pub fn width(label: Option<String>) -> usize {
    label.map(|s| s.len()).unwrap_or(0)
}
