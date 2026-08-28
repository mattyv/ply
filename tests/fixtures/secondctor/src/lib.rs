//! docs/review-silent-narrowing.md finding 3, 2026-08-28: `TwoCtor` has two
//! usable constructors, `new` and `preloaded`, and Ply's receiver scan
//! always builds a receiver by calling exactly one of them (the first
//! fully-buildable one found, in source order) -- so every case this run
//! generates starts from `new`, and states reachable only through
//! `preloaded` are never explored:
//!
//! ```ignore
//! TwoCtor::preloaded(7).value() // -> 7, not 0
//! ```
//!
//! `value`'s promise (always 0) is genuinely false for anything built by
//! `preloaded`, and this run genuinely cannot find that by running cases,
//! because it never calls `preloaded`. What this fixture pins is the
//! disclosure: `preloaded` named as a constructor this run never started
//! from, and the verdict marked narrower than a bare `fuzzed(n)` reads.

pub struct TwoCtor {
    n: u32,
}

impl TwoCtor {
    pub fn new() -> Self {
        TwoCtor { n: 0 }
    }

    pub fn preloaded(n: u32) -> Self {
        TwoCtor { n }
    }

    #[ply::ensures(|result| *result == 0)]
    pub fn value(&self) -> u32 {
        self.n
    }
}
