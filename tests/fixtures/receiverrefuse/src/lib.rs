//! Receiver construction's refusal-by-name half
//! (docs/review-self-construction.md's "fourth option", task 2026-08-27):
//! a type Ply cannot build a receiver for must be refused, naming why --
//! never guessed at, never filled in field by field.

/// No associated function on `Gauge` returns `Self` at all -- there is
/// nothing for Ply to build a receiver by calling. Must be refused by name,
/// naming `Gauge`.
pub struct Gauge {
    n: u32,
}

impl Gauge {
    #[ply::ensures(|result| *result == *result)]
    pub fn read(&self) -> u32 {
        self.n
    }
}

/// Stands in for "a type Ply's checkers cannot build a value of".
///
/// Its field is PRIVATE and it has no constructor of its own, so neither
/// route can produce one: there is nothing to call, and nothing outside
/// this module could write the literal either. It was a public-field struct
/// until 2026-08-28, when struct parameters landed and made that shape
/// buildable -- at which point this fixture stopped testing what its own
/// name says and started asserting a limitation that no longer existed. The
/// capability grew and the fixture did not, which is worth a comment rather
/// than a silent edit: a test that pins an absence has to be re-checked
/// every time the absence might have ended.
pub struct Tag {
    label: u32,
}

/// `Labelled`'s only constructor takes a `Tag`, which Ply's checkers cannot
/// build a value of -- the constructor itself can never be called, so the
/// refusal must name `Tag`, not merely say "no constructor".
pub struct Labelled {
    tag: Tag,
}

impl Labelled {
    pub fn new(tag: Tag) -> Self {
        Labelled { tag }
    }

    #[ply::ensures(|result| *result == *result)]
    pub fn tag_value(&self) -> u32 {
        self.tag.label
    }
}

/// Constructible, and `bump` takes `&mut self`. This used to be refused,
/// and the reason given was that Ply "has no way yet to state what a
/// `&mut self` call is supposed to change about the receiver". **That was
/// wrong and is retracted (2026-09-07):** a promise says it in terms of the
/// value's own readings before and after. `bump` is kept here, carrying no
/// promise of its own, so this fixture pins that the receiver is no longer
/// what stops it -- a test that pins an absence has to be re-checked every
/// time the absence might have ended, which is exactly what the `Tag`
/// comment above says and exactly what nearly went unnoticed here.
pub struct Counter {
    n: u32,
}

impl Counter {
    pub fn new() -> Self {
        Counter { n: 0 }
    }

    pub fn bump(&mut self) {
        self.n += 1;
    }
}

/// Constructible, but `into_total` takes `self` **by value**: calling it
/// consumes the value, so a receiver Ply built cannot be called into again.
/// This is the one receiver shape still refused, and it is refused as a
/// second piece of codegen that does not exist rather than as something
/// that cannot be said -- so it is pinned here, end to end, now that the
/// `&mut self` refusal it used to share this fixture with has gone.
pub struct Ledger {
    total: u32,
}

impl Ledger {
    pub fn new() -> Self {
        Ledger { total: 0 }
    }

    #[ply::ensures(|result| *result == *result)]
    pub fn into_total(self) -> u32 {
        self.total
    }
}
