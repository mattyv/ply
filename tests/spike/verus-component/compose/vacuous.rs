// A contradictory premise set discharges every obligation and means
// nothing. This is the sibling hole to the arithmetic one: the non-vacuity
// checks in FINDINGS.md tested sensitivity to two weakenings, which is a
// different property and does not cover this.
//
// `refill` is given two postconditions that cannot both hold. Expect
// "verified, 0 errors" -- and that result is worthless.
use vstd::prelude::*;

verus! {

pub struct S { pub available: int, pub capacity: int }

pub open spec fn inv(s: S) -> bool { s.available <= s.capacity }

pub open spec fn contradictory_refill(pre: S, post: S) -> bool {
    post.available == pre.available + 1 && post.available == pre.available
}

/// Discharges for the wrong reason: the premises are unsatisfiable, so the
/// implication is vacuously true for every state including ones that break
/// the invariant.
proof fn ob_preserve_refill(pre: S, post: S)
    requires inv(pre), contradictory_refill(pre, post)
    ensures inv(post)
{ }

fn main() { }
}
