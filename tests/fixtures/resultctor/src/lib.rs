//! A violation reported on correct code (docs/review-structs-enums.md
//! finding 2, 2026-08-28): `Range::new` is the ordinary fallible-constructor
//! shape in Rust -- it returns `Result<Self, E>` and rejects the state
//! `well_formed`'s own promise depends on nothing ever reaching. Because
//! Ply did not recognise a `Result`-returning constructor as a constructor
//! at all, it fell through to direct field construction, built `Range { lo:
//! 8, hi: 0 }` -- the exact state `new` exists to forbid -- and reported
//! `well_formed`, correct code, as breaking its own promise.

pub struct Range {
    pub lo: u32,
    pub hi: u32,
}

impl Range {
    /// Rejects `lo > hi` -- the ordinary way a fallible constructor is
    /// written in Rust, not an exotic shape.
    pub fn new(lo: u32, hi: u32) -> Result<Self, String> {
        if lo > hi {
            return Err("bad range".into());
        }
        Ok(Self { lo, hi })
    }
}

/// TRUE of every `Range` the real program can build -- `Range::new` is the
/// only public way to construct one, and it never returns a value with
/// `lo > hi`.
#[ply::ensures(|result| *result)]
pub fn well_formed(w: Range) -> bool {
    w.lo <= w.hi
}
