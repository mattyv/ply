//! Blocker 3 (task 2026-08-27, docs/review-strings-receivers.md finding 3):
//! "Ply breaks the user's own build." A receiver method whose promise is
//! false on every input, checked by `fuzz(n)`. Before the fix, the rendered
//! replay test called `Gauge::level()` with no receiver at all (`Gauge`
//! takes `&self`), which does not compile -- and Ply had already added a
//! `mod` line to this crate's own `lib.rs` pointing at it, so the user's own
//! `cargo test` stopped building. This crate must stay buildable no matter
//! what `cargo ply verify` finds.

pub struct Gauge {
    pub n: u32,
}

impl Gauge {
    pub fn new(n: u32) -> Self {
        Gauge { n }
    }

    /// FALSE on every input: `level()` just returns `n`, which can be
    /// anything `new` was given -- `== 999999` never holds in general.
    #[ply::ensures(|result| *result == 999999)]
    pub fn level(&self) -> u32 {
        self.n
    }
}
