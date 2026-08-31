//! Defect 2, found pointing Ply at `semver` (docs/reach-measurement-2.md):
//! a method whose parameter shares its receiver's type generates a harness
//! that imports the same type twice -- once to build the receiver, once to
//! build the parameter -- and `use Pair;` twice in one module is
//! `error[E0252]: the name `Pair` is defined multiple times`. Every method
//! taking another value of its own type has this shape (`merge`, `cmp`,
//! `min`/`max`, `same_as` in the measurement's own report): an extremely
//! common one.
//!
//! Two more shapes that can produce the same duplicate `use`, covered here
//! too: two *parameters* of the same type on one receiverless function
//! (`both_same`), and a parameter whose type is also the checked method's
//! own return type (`copy_of`, returning `Self` -- the receiver's type named
//! three times over in one signature).
//!
//! Two adjustments below, neither one this defect, both found while writing
//! this fixture and recorded rather than fixed, per this task's own scope
//! rule:
//!
//! - no postcondition below reads `self`. `#[ply::ensures]` on a receiver
//!   method is generated as a free function's assertion body, and nothing
//!   rewrites a bare `self` in it to the actual receiver binding -- so
//!   `self.a == other.a` renders as literal `self.a`, which does not exist
//!   outside an `impl` block: `error[E0424]: expected value, found module
//!   `self``. No fixture in this crate exercised that path before, so it
//!   was invisible until this task's own reproduction (the measurement's
//!   `same_as(&self, other: &Pair)`, verbatim, reads `self.a`) tried it.
//! - every boolean-returning postcondition states its `==`/`!=` as an `iff`
//!   written with `&&`/`||` (`(!*result || lhs == rhs) && (*result || lhs
//!   != rhs)`) rather than `*result == (lhs == rhs)` -- logically identical,
//!   but the latter is a separate pre-existing defect: Ply's postcondition
//!   widening casts a nested comparison's *last operand alone* to `i128`
//!   rather than the whole comparison, so `*result == (a.a == b.a)` renders
//!   as `a.a == (b.a as i128)`, comparing `u64` to `i128` -- `error[E0308]`.

pub struct Pair {
    pub a: u64,
}

impl Pair {
    pub fn new(a: u64) -> Self {
        Pair { a }
    }

    /// The measurement's own reproduction shape: a `&self` method taking
    /// another `&Pair` -- the receiver's own type, imported once for the
    /// receiver and, before this fix, a second time for the parameter.
    #[ply::ensures(|result| *result == other.a)]
    pub fn value_of(&self, other: &Pair) -> u64 {
        other.a
    }

    /// The receiver's own type named three times over in one signature --
    /// as the receiver, as `other`'s parameter type, and as the return type
    /// (`Self`) -- the shape `Ord::max`/`Ord::min` have in real code.
    #[ply::ensures(|result| result.a == other.a)]
    pub fn copy_of(&self, other: &Pair) -> Self {
        Pair { a: other.a }
    }
}

/// Two parameters sharing one struct type, no receiver involved -- the same
/// duplicate-`use` shape can arise between two parameters alone.
#[ply::ensures(|result| (!*result || a.a == b.a) && (*result || a.a != b.a))]
pub fn both_same(a: &Pair, b: &Pair) -> bool {
    a.a == b.a
}
