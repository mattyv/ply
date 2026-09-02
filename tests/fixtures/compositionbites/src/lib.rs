//! Composition (TODO.md, "make the sampling engine's decision recursive",
//! 2026-09-02) must not just refuse *less* -- turning every one of these
//! four refusals into a comfortable green would be worse than the defect
//! it fixes. Three of the four shapes this task's own probe measured as
//! refused live here, each with a promise that is genuinely false, so a
//! real run must find a real violation with a real failing input. The
//! fourth (an optional string) is proven the same way in
//! `paramseeded_fixture.rs`.

/// A list of strings: promise says the longest one is at most 10
/// characters, which real sampled text (up to 32 characters) breaks.
#[ply::ensures(|result| *result <= 10)]
pub fn longest_len(xs: Vec<String>) -> usize {
    xs.iter().map(|s| s.chars().count()).max().unwrap_or(0)
}

/// A slice: promise says the sum never exceeds 100, which eight sampled
/// bytes easily break.
#[ply::ensures(|result| *result <= 100)]
pub fn sum_slice(xs: &[u32]) -> u32 {
    xs.iter().sum()
}

pub struct Item {
    n: u32,
}

impl Item {
    pub fn new(n: u32) -> Self {
        Item { n }
    }
}

/// A nested user struct: promise says the total never exceeds 10, which a
/// handful of sampled `Item`s easily break.
#[ply::ensures(|result| *result <= 10)]
pub fn total_n(items: Vec<Item>) -> u32 {
    items.iter().map(|i| i.n).sum()
}
