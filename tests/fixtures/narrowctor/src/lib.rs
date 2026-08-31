//! Defect (adversarial review, 2026-08-31, "a fallible constructor's
//! rejection arm can be turned into a vacuous pass and nothing notices"):
//! the receiver test fixture (`receiverresultctor`) only ever exercises a
//! fallible constructor that rejects a single value (`v == 0`) with a
//! promise that is vacuously true for any `u64` (`*result >= 0`). Turning
//! that constructor's rejection arm into `Ok(Self { v })` (never actually
//! rejecting) does not change that fixture's outcome at all: the promise
//! still holds regardless, so the mutation goes unnoticed.
//!
//! `Narrow::new` here rejects most of its input domain (anything above 3,
//! against a generator that draws mostly from 0..=16) and `doubled`'s
//! promise is not vacuous -- it only holds because the constructor's
//! rejection actually narrows the receiver to `0..=3`. If the rejection
//! arm were ever turned into a vacuous pass, out-of-domain receivers would
//! reach `doubled` and its promise would break; with the rejection working
//! as written, most draws are thrown away and the ones that remain are
//! real evidence that the high-rejection warning exists to flag.

pub struct NarrowCtorErr;

pub struct Narrow {
    pub v: u64,
}

impl Narrow {
    pub fn new(v: u64) -> Result<Self, NarrowCtorErr> {
        if v > 3 { Err(NarrowCtorErr) } else { Ok(Self { v }) }
    }

    #[ply::ensures(|result| *result <= 6)]
    pub fn doubled(&self) -> u64 {
        self.v.saturating_mul(2)
    }
}
