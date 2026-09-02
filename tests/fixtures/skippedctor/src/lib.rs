//! Is the disclosure enough? (docs/review-structs-enums.md finding 2,
//! 2026-08-28): `Point` has a constructor Ply's scan finds, but cannot use
//! -- `labeled`'s own `_label` argument is a type the fuzz tier cannot
//! build. That argument was a `&str` until 2026-09-01, when borrowed text
//! became buildable, and `Option<String>` until 2026-09-02, when
//! composition (TODO.md) made a `String` buildable no matter how it nests
//! -- so both stopped making this constructor unusable, and the argument is
//! now `&mut u32`, a shape that stays refused for a structural reason
//! composition does not touch (§5.4b stops at a shared `&T`; a value the
//! function writes back through is not one either engine can construct and
//! observe), so the fixture goes on testing the disclosure it was written
//! for rather than a capability Ply has since gained. Ply correctly falls
//! through to rule 2 (direct field construction: every field of `Point` is
//! public, so there is no hidden invariant to violate here) -- but the
//! W0522 disclosure that fires must say a constructor was found and
//! skipped, and why, rather than reading as though direct construction were
//! the only route this type ever offered.

pub struct Point {
    pub x: u32,
    pub y: u32,
}

impl Point {
    /// A real constructor, but Ply cannot build its `&mut u32` argument --
    /// so rule 1 fails for this candidate even though a candidate exists.
    pub fn labeled(_label: &mut u32, x: u32, y: u32) -> Self {
        Point { x, y }
    }
}

/// Trivially true for a `u32` field -- this fixture is about the
/// disclosure, not about finding a bug.
#[ply::ensures(|result| *result)]
pub fn always_nonneg(p: Point) -> bool {
    p.x >= 0
}
