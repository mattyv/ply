//! Struct and enum **parameters** (this task, 2026-08-27):
//! `docs/review-self-construction.md` settled how a value of the user's own
//! type may be built for a receiver (`ReceiverPlan`: the type's own
//! constructor, honouring its own precondition, plus an optional bounded
//! sequence of its own operations); this fixture is for the same rule
//! applied to an *ordinary* parameter instead of `&self`. In order:
//!
//! 1. construct via the type's own constructor where one exists that takes
//!    buildable arguments (`TicketPool`, `Bucket`);
//! 2. direct field/variant construction only when every field (`Point`) or
//!    every variant's every field (`Shape`) is already public;
//! 3. otherwise refuse by name (`Locked`).
//!
//! Every fn below is checked with `fuzz(n)` -- the sampling tier is the
//! only one this shape reaches: Kani's harness codegen was never built for
//! a constructor call or a struct/enum literal (`harness.rs`'s own module
//! doc).

// -- Rule 2: an all-public-fields struct -------------------------------------
// No invariant to violate: any caller could already build any `Point`.

#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[ply::ensures(|result| *result >= 0)]
pub fn manhattan_norm(p: Point) -> i64 {
    (p.x as i64).abs() + (p.y as i64).abs()
}

// -- Rule 2 for an enum: every variant's own fields are public --------------

#[derive(Debug, Clone, Copy)]
pub enum Shape {
    Circle { radius: u32 },
    Square { side: u32 },
    Origin,
}

#[ply::ensures(|result| *result >= 0)]
pub fn shape_area_upper_bound(s: Shape) -> i64 {
    // Widths only, deliberately -- no squaring, so this never overflows
    // `i64` for any `u32` (this fixture is about enum *construction*, not
    // arithmetic; a real bug's plumbing is `broken_doubled_capacity`,
    // below).
    match s {
        Shape::Circle { radius } => radius as i64,
        Shape::Square { side } => side as i64,
        Shape::Origin => 0,
    }
}

// -- Rule 1: a private-field struct, built only through its own constructor --

#[derive(Debug, Clone, Copy)]
pub struct TicketPool {
    capacity: u32,
}

impl TicketPool {
    pub fn new(capacity: u32) -> Self {
        TicketPool { capacity }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}

/// A real, passing check: doubling a capacity is always even.
#[ply::ensures(|result| *result % 2 == 0)]
pub fn doubled_capacity(p: TicketPool) -> u64 {
    p.capacity() as u64 * 2
}

/// **The decisive test**: a genuinely broken function taking a struct built
/// via its own constructor. The promise is FALSE on every input -- a
/// passing check here would prove nothing; this must be CAUGHT as a
/// `violation`.
#[ply::ensures(|result| *result == 999999)]
pub fn broken_doubled_capacity(p: TicketPool) -> u64 {
    p.capacity() as u64 * 2
}

// -- The other decisive test: a constructor-maintained invariant must never
// be handed an impossible value -----------------------------------------

/// Both fields are private, so rule 2 (direct field construction) does not
/// apply -- Ply can only use the constructor (rule 1). `new` always starts
/// a bucket full (`tokens == capacity`). If Ply had instead filled these
/// private fields in directly, it could build `Bucket { capacity: 1,
/// tokens: 999 }` -- a state the real program can never produce -- and the
/// check below would fail on a false alarm. Because it cannot, every
/// generated `Bucket` really does start full, and the check must pass
/// cleanly.
#[derive(Debug, Clone, Copy)]
pub struct Bucket {
    capacity: u32,
    tokens: u32,
}

impl Bucket {
    pub fn new(capacity: u32) -> Self {
        Bucket {
            capacity,
            tokens: capacity,
        }
    }
}

#[ply::ensures(|result| *result)]
pub fn tokens_never_exceed_capacity(b: Bucket) -> bool {
    b.tokens <= b.capacity
}

// -- Rule 3: refused by name --------------------------------------------

/// No associated function returns bare `Self` at all (rule 1 fails), and
/// its one field is private (rule 2 fails too) -- refused by name.
#[derive(Debug, Clone, Copy)]
pub struct Locked {
    secret: u32,
}

impl Locked {
    pub fn secret(&self) -> u32 {
        self.secret
    }
}

#[ply::ensures(|result| *result >= 0)]
pub fn read_secret(l: Locked) -> u32 {
    l.secret()
}

// -- Everything currently refused for other reasons stays refused ----------
// (a `&mut` parameter, regardless of struct/enum support -- unrelated to
// this task, pinned here so a future change to this fixture cannot quietly
// stop exercising it).

#[ply::ensures(|result| *result >= 0)]
pub fn bump_mut(x: &mut u32) -> u32 {
    *x += 1;
    *x
}
