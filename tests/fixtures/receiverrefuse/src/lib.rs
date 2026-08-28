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

/// Constructible (`Counter::new` needs nothing Ply cannot build), but
/// `bump` takes `&mut self` -- still refused, unchanged by this task: Ply
/// has no way yet to state what a `&mut self` call is supposed to change
/// about the receiver, so a built receiver would not be enough on its own.
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
