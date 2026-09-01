//! The fourteenth false clean, closed. `docs/review-structs-enums.md`
//! finding 1 recorded a real bug Ply could not find: `note` is the only
//! operation that changes `Acc`'s state, it takes borrowed text, and until
//! 2026-09-01 Ply could not build a text argument. So `note` vanished from
//! the pool of operations used to build a receiver, every generated case
//! called `get` on a receiver only the constructor had touched, and `get`'s
//! promise (always 0) came back a clean pass -- on code that breaks after
//! one ordinary call:
//!
//! ```ignore
//! let mut a = Acc::new();
//! a.note("x");
//! a.get() // -> 5, not 0
//! ```
//!
//! Borrowed text became buildable that day, measured as the single largest
//! reason Ply could not check a real library. This fixture is the smallest
//! thing that proves the consequence: Ply now calls `note`, reaches the
//! state that breaks the promise, and reports a `violation` where it used
//! to report a pass.
//!
//! Its sibling `excludedop` keeps the other half honest -- there `note`
//! takes a shape that is still unbuildable, so the run still cannot call
//! it, and must say so rather than claiming a clean history.

pub struct Acc {
    total: u32,
}

impl Acc {
    pub fn new() -> Self {
        Acc { total: 0 }
    }

    /// The only way `Acc`'s state ever changes. Ply can build a text
    /// argument now, so this is reachable.
    pub fn note(&mut self, _s: &str) {
        self.total += 5;
    }

    /// FALSE after a single `note` call, and Ply now finds that by running
    /// cases rather than reporting a pass it did not earn.
    #[ply::ensures(|result| *result == 0)]
    pub fn get(&self) -> u32 {
        self.total
    }
}
