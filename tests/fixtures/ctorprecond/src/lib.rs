//! Adversarial review, docs/review-caveats.md N2: Ply built a receiver by
//! calling the type's own constructor with an argument the constructor's
//! own declared precondition forbids, so the constructor's own assertion
//! fired -- and the crash was reported as `Gauge::value` breaking its own
//! promise. `*result >= 0` on a `u32` cannot be false: this fixture pins
//! that a promise no unsigned integer can break must never come back as a
//! `violation` because Ply itself called `Gauge::new(0)`, a call `new`'s
//! own `#[ply::requires(n > 0)]` forbids.

pub struct Gauge {
    n: u32,
}

impl Gauge {
    #[ply::requires(n > 0)]
    pub fn new(n: u32) -> Self {
        assert!(n > 0, "a Gauge reading is never zero");
        Gauge { n }
    }

    /// Cannot be false: `n` is a `u32`, so `*result >= 0` holds for every
    /// value `new` could ever produce.
    #[ply::ensures(|result| *result >= 0)]
    pub fn value(&self) -> u32 {
        self.n
    }
}
