//! A violation reported on correct code, second shape
//! (docs/review-structs-enums.md finding 2, 2026-08-28): `Window` is
//! declared in `types.rs`; its constructor is written in `build.rs`, an
//! ordinary way to organise a Rust crate. Ply used to search for a
//! constructor only in the file where the type itself is declared, so this
//! layout hid `Window::new` -- and its own `#[ply::requires]`, written in
//! Ply's own notation -- entirely, falling through to direct field
//! construction and reporting `well_formed`, correct code, as broken.

pub mod build;
pub mod types;

pub use types::Window;

/// TRUE of every `Window` the real program can build -- `Window::new` is
/// the only public way to construct one, and its own precondition forbids
/// `start > end`.
#[ply::ensures(|result| *result)]
pub fn well_formed(w: Window) -> bool {
    w.start <= w.end
}
