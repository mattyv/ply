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

/// Stands in for "a type Ply's parser does not model at all" -- the same
/// role `Elsewhere` plays in `tests/fixtures/implmethod`.
pub struct Tag {
    pub label: u32,
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
