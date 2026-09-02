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
//! `note` took a `&str` until 2026-09-01, when borrowed text became a
//! buildable argument, and `Option<String>` until 2026-09-02, when
//! composition (TODO.md) made a `String` buildable no matter how it nests
//! -- both stopped making `note` uncallable, so this fixture is preserved by
//! giving `note` a parameter still unbuildable for a structural reason
//! composition does not touch: `&mut u32` (§5.4b stops at a shared `&T`; a
//! value the function writes back through is not one either engine can
//! construct and observe). The companion fixture `textmutator` records the
//! other half: the same shape with a `&str`, where Ply now finds the
//! violation it used to miss.
//!
//! So this run genuinely cannot
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
    /// `&mut u32` argument to call it with.
    pub fn note(&mut self, _s: &mut u32) {
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
