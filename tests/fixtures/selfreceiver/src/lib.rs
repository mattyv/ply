//! Defect 1, found pointing Ply at `semver` (docs/reach-measurement-2.md):
//! a method's own postcondition could not mention the receiver it is
//! called on -- `#[ply::ensures]` is spliced into the generated harness as
//! a free-standing expression outside any `impl` block, where the literal
//! keyword `self` means nothing (`error[E0424]: expected value, found
//! module `self``). This is the single most natural thing a method's own
//! promise can say: almost every useful postcondition on a method relates
//! its result to the receiver it was called on.
//!
//! Three shapes, all fixed by rewriting a bare `self` in the postcondition
//! to the receiver binding the generated harness already builds:
//!
//! - `self` and the result together (`Pair::bumped`) -- the reported repro,
//!   verbatim.
//! - `self` and a parameter together (`Pair::at_least`).
//! - a receiver built through a fallible (`Result<Self, E>`) constructor,
//!   whose own postcondition also reads `self` (`Meter::doubled`) -- this
//!   interacts with the *other* defect fixed the same day
//!   (`receiverresultctor`): the receiver scan must still recognise this
//!   constructor, in the same run whose postcondition now also has to read
//!   the receiver it builds.

pub struct Pair {
    pub a: u64,
}

impl Pair {
    pub fn new(a: u64) -> Self {
        Pair { a }
    }

    /// The reported repro, verbatim: `self` and the result together.
    #[ply::ensures(|result| *result >= self.a)]
    pub fn bumped(&self) -> u64 {
        self.a.saturating_add(1)
    }

    /// `self` and a parameter together.
    #[ply::ensures(|result| *result >= self.a && *result >= extra)]
    pub fn at_least(&self, extra: u64) -> u64 {
        self.a.saturating_add(extra)
    }
}

pub struct MeterErr;

pub struct Meter {
    pub n: u64,
}

impl Meter {
    /// A fallible constructor -- the shape `receiverresultctor` fixed the
    /// receiver scan for. Must still be found here, in the same run whose
    /// postcondition also reads `self`.
    pub fn new(n: u64) -> Result<Self, MeterErr> {
        if n == 0 { Err(MeterErr) } else { Ok(Meter { n }) }
    }

    #[ply::ensures(|result| *result >= self.n)]
    pub fn doubled(&self) -> u64 {
        self.n.saturating_mul(2)
    }
}
