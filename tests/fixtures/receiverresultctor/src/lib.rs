//! Defect 1, found pointing Ply at `semver` (docs/reach-measurement-2.md):
//! `A::doubled` is an ordinary `&self` method, and `A::new` is a public,
//! fully-buildable constructor for `A` -- `read_it` below builds an `A`
//! *parameter* by calling exactly that constructor, in the very same run
//! that (before this fix) reported `A::doubled`'s receiver as unbuildable,
//! naming `A` as a type with "no associated function ... that builds a `A`
//! value". Same type, same constructor, same file, same run: the parameter
//! path already knew how to call it, and the receiver path's own
//! constructor scan simply never learned the same widening.
//!
//! `Self` vs. the type's own name, and bare vs. `Result`-wrapped, are four
//! spellings of one constructor shape. Each of the four struct types below
//! writes it a different way; every one must be found and called by the
//! receiver path exactly as the parameter path already finds and calls it.

pub struct CtorErr;

/// `-> Self`.
pub struct BareSelf {
    pub v: u64,
}
impl BareSelf {
    pub fn new(v: u64) -> Self {
        BareSelf { v }
    }
    #[ply::ensures(|result| *result >= 0)]
    pub fn doubled(&self) -> u64 {
        self.v.saturating_mul(2)
    }
}

/// `-> ExplicitName` -- the type's own name written out, ordinary Rust and
/// no different in meaning from `-> Self` inside this same `impl` block.
pub struct ExplicitName {
    pub v: u64,
}
impl ExplicitName {
    pub fn new(v: u64) -> ExplicitName {
        ExplicitName { v }
    }
    #[ply::ensures(|result| *result >= 0)]
    pub fn doubled(&self) -> u64 {
        self.v.saturating_mul(2)
    }
}

/// `-> Result<Self, CtorErr>` -- the ordinary fallible-constructor shape;
/// rejects `v == 0`.
pub struct ResultSelf {
    pub v: u64,
}
impl ResultSelf {
    pub fn new(v: u64) -> Result<Self, CtorErr> {
        if v == 0 { Err(CtorErr) } else { Ok(Self { v }) }
    }
    #[ply::ensures(|result| *result >= 0)]
    pub fn doubled(&self) -> u64 {
        self.v.saturating_mul(2)
    }
}

/// `-> Result<ResultExplicitName, CtorErr>` -- the same fallible shape,
/// spelling the type's own name instead of `Self`.
pub struct ResultExplicitName {
    pub v: u64,
}
impl ResultExplicitName {
    pub fn new(v: u64) -> Result<ResultExplicitName, CtorErr> {
        if v == 0 {
            Err(CtorErr)
        } else {
            Ok(ResultExplicitName { v })
        }
    }
    #[ply::ensures(|result| *result >= 0)]
    pub fn doubled(&self) -> u64 {
        self.v.saturating_mul(2)
    }
}

/// The measurement's own reproduction, verbatim in shape: `A::new` returns
/// `Result<A, Bad>`, and `read_it` below builds an `A` *parameter* by
/// calling it -- in the same run that must now also build `A` as
/// `A::doubled`'s receiver, through the very same constructor.
pub struct Bad;

pub struct A {
    pub v: u64,
}
impl A {
    pub fn new(v: u64) -> Result<A, Bad> {
        if v == 0 { Err(Bad) } else { Ok(A { v }) }
    }
    #[ply::ensures(|result| *result >= 0)]
    pub fn doubled(&self) -> u64 {
        self.v.saturating_mul(2)
    }
}

#[ply::ensures(|result| *result >= 0)]
pub fn read_it(a: A) -> u64 {
    a.v
}
