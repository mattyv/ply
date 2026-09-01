//! Is the disclosure enough? (docs/review-structs-enums.md finding 2,
//! 2026-08-28): `Point` has a constructor Ply's scan finds, but cannot use
//! -- `labeled`'s own `_label` argument is a type the fuzz tier cannot
//! build. That argument was a `&str` until 2026-09-01, when borrowed text
//! became buildable and this constructor stopped being unusable; it is now
//! an `Option<String>` (a `String` nested inside another type, deliberately
//! never built) so the fixture goes on testing the disclosure it was
//! written for rather than a capability Ply has since gained. Ply correctly falls through to rule 2 (direct field
//! construction: every field of `Point` is public, so there is no hidden
//! invariant to violate here) -- but the W0522 disclosure that fires must
//! say a constructor was found and skipped, and why, rather than reading as
//! though direct construction were the only route this type ever offered.

pub struct Point {
    pub x: u32,
    pub y: u32,
}

impl Point {
    /// A real constructor, but Ply cannot build its `Option<String>`
    /// argument -- so rule 1 fails for this candidate even though a
    /// candidate exists.
    pub fn labeled(_label: Option<String>, x: u32, y: u32) -> Self {
        Point { x, y }
    }
}

/// Trivially true for a `u32` field -- this fixture is about the
/// disclosure, not about finding a bug.
#[ply::ensures(|result| *result)]
pub fn always_nonneg(p: Point) -> bool {
    p.x >= 0
}
