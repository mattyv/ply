//! The fourteenth false clean (docs/review-structs-enums.md finding 1,
//! 2026-08-28): `note` is the only operation that ever changes `Acc`'s
//! state, and it takes a `&str` -- a type Ply's fuzz tier cannot build an
//! argument for. Before this fixture's fix, `note` simply vanished from
//! the receiver's bounded operation pool with nothing said about it, so
//! every generated case called `get` on a receiver only the constructor had
//! ever touched, and `get`'s promise (always 0) reported a clean
//! `fuzzed(n)` pass forever -- even though the real program breaks it after
//! one ordinary call:
//!
//! ```ignore
//! let mut a = Acc::new();
//! a.note("x");
//! a.get() // -> 5, not 0
//! ```
//!
//! Ply still cannot build a `&str` argument (a separate, known gap -- see
//! docs/review-structs-enums.md finding 5), so this run genuinely cannot
//! call `note` and genuinely cannot find this violation by running cases.
//! What it must do instead is say so: name `note` as an operation this run
//! never called, and mark the verdict as resting on a narrower history than
//! an unqualified `fuzzed(n)` would suggest -- never claim, as the old
//! wording did, that "nothing here was assumed".

pub struct Acc {
    total: u32,
}

impl Acc {
    pub fn new() -> Self {
        Acc { total: 0 }
    }

    /// The only way `Acc`'s state ever changes -- and Ply cannot build a
    /// `&str` argument to call it with.
    pub fn note(&mut self, _s: &str) {
        self.total += 5;
    }

    /// FALSE after a single `note` call. A receiver sequence that can never
    /// include `note` can never see this fail -- which is exactly why the
    /// run must say so rather than reporting a clean pass indistinguishable
    /// from one that actually explored `note`'s effect.
    #[ply::ensures(|result| *result == 0)]
    pub fn get(&self) -> u32 {
        self.total
    }
}
