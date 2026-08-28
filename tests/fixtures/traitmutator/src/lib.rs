//! docs/review-silent-narrowing.md finding 2, 2026-08-28: `TraitTill`'s
//! only mutating operation, `take`, is a *trait* method (`impl Fill for
//! TraitTill`), never an inherent one. The receiver scan skips trait impls
//! outright -- genuinely out of scope, not merely unwidened -- so `take`
//! can never enter `TraitTill::total`'s operation pool and this run
//! genuinely cannot find that `total`'s promise (always 0) is false after
//! one ordinary `take` call:
//!
//! ```ignore
//! let mut t = TraitTill::new();
//! t.take(5);
//! t.total() // -> 5, not 0
//! ```
//!
//! What this fixture pins is not that Ply finds the bug (it cannot, and
//! that is a real, permanent limit: calling through a trait needs the
//! trait itself in scope) but that it says so honestly -- naming `take` as
//! excluded because it is a trait method, and marking the verdict narrower
//! than a bare `fuzzed(n)` reads, rather than claiming completeness it
//! never earned.

pub trait Fill {
    fn take(&mut self, cents: u32) -> u32;
}

pub struct TraitTill {
    total: u32,
}

impl TraitTill {
    pub fn new() -> Self {
        TraitTill { total: 0 }
    }

    #[ply::ensures(|result| *result == 0)]
    pub fn total(&self) -> u32 {
        self.total
    }
}

impl Fill for TraitTill {
    fn take(&mut self, cents: u32) -> u32 {
        self.total = self.total.saturating_add(cents);
        self.total
    }
}
