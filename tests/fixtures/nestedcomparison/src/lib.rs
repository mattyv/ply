//! Defect 2, found pointing Ply at `semver` (docs/reach-measurement-2.md):
//! a comparison nested *inside* another comparison as a leaf did not
//! compile. `contract_rt::widen`'s catch-all leaf case cast a whole nested
//! comparison's token stream to `i128` without wrapping it in its own
//! parens first, so `*result == (a == b)` rendered as `a == b as i128` --
//! because `as` binds tighter than `==`, that compares `u64` to `i128`
//! (`error[E0308]`) instead of casting the whole comparison.
//!
//! `same` below is the reported repro, verbatim. Four more shapes, split
//! by whether they actually exercise the defect (a comparison nested
//! *inside another comparison* -- reaches `widen_leaf`'s catch-all as an
//! opaque leaf) or not (two comparisons combined directly by a top-level
//! `&&`/`||` with no wrapping equality -- `widen`'s own `&&`/`||`
//! recursion already calls `widen` on each side directly, never touching
//! the leaf path at all):
//!
//! - needed the fix: `both_eq`/`either_eq` (a comparison nested under
//!   `&&`/`||`, that whole thing then wrapped by an outer `==`),
//!   `halves_eq` (a comparison of two expressions, not bare names),
//!   `bumped_eq` (arithmetic on one side of the nested comparison).
//! - already worked, unchanged by this fix: `bigger`/`either` (`&&`/`||`
//!   as the postcondition's own outermost operator, each side already its
//!   own comparison -- confirmed by running both against the pre-fix
//!   binary).

/// The reported repro, verbatim.
#[ply::ensures(|result| *result == (a == b))]
pub fn same(a: u64, b: u64) -> bool {
    a == b
}

/// A comparison nested under `&&`, that whole conjunction then wrapped by
/// an outer `==` -- needed the fix.
#[ply::ensures(|result| *result == (a == b && c == d))]
pub fn both_eq(a: u64, b: u64, c: u64, d: u64) -> bool {
    a == b && c == d
}

/// A comparison nested under `||`, likewise wrapped by an outer `==` --
/// needed the fix.
#[ply::ensures(|result| *result == (a == b || c == d))]
pub fn either_eq(a: u64, b: u64, c: u64, d: u64) -> bool {
    a == b || c == d
}

/// A comparison of two expressions, not two bare names -- needed the fix.
#[ply::ensures(|result| *result == (a / 2 == b / 2))]
pub fn halves_eq(a: u64, b: u64) -> bool {
    a / 2 == b / 2
}

/// Arithmetic on one side of the nested comparison -- needed the fix.
/// `requires` keeps `a + 1` from overflowing, both in the real body and in
/// the widened check.
#[ply::requires(a < u64::MAX)]
#[ply::ensures(|result| *result == (a + 1 == b))]
pub fn bumped_eq(a: u64, b: u64) -> bool {
    a + 1 == b
}

/// `&&` as the postcondition's own outermost operator, each side already
/// its own top-level comparison -- `widen`'s existing recursion handles
/// this directly. Already worked before this fix.
#[ply::ensures(|result| *result >= a && *result >= b)]
pub fn bigger(a: u64, b: u64) -> u64 {
    if a > b { a } else { b }
}

/// `||` as the postcondition's own outermost operator, same reasoning --
/// already worked before this fix.
#[ply::ensures(|result| *result == a || *result == b)]
pub fn either(a: u64, b: u64, pick_a: bool) -> u64 {
    if pick_a { a } else { b }
}
