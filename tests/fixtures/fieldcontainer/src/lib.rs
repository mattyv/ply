//! A container of a user-defined struct or enum, nested one level below a
//! parameter's own top level -- a struct's own field, an enum variant's own
//! field, or a constructor's own argument, rather than the parameter
//! itself. Composition (2026-09-02) already made `Vec<Item>` buildable as a
//! bare *top-level* parameter (`compositionbites::total_n`); this fixture
//! is the shape one level deeper, which turned out to have its own,
//! separate hole (TODO.md, "finish the container fix", 2026-09-04): a
//! struct with such a field crashed Ply outright before the field-scanning
//! gate looked inside it, and the container-walking resolver a top-level
//! parameter already used through was never routed through for a field, a
//! variant field, or a constructor argument. `record::fingerprint`'s own
//! `FingerprintInputs` has two fields of exactly this shape (`assumed:
//! Vec<AssumedPromise>`, `engines: Vec<EngineId>`).
//!
//! Every promise below is genuinely false, so a real run has to catch it
//! with a real failing input -- turning a former refusal into a comfortable
//! green would be worse than the defect this fixture exists to pin.

pub struct Item {
    pub n: u32,
}

/// Rule 2, direct construction: every field public, including the
/// `Vec<Item>` field.
pub struct Bag {
    pub items: Vec<Item>,
}

/// Eight sampled `Item`s (proptest's own `Vec` bound) easily push the sum
/// past 10.
#[ply::ensures(|result| *result <= 10)]
pub fn bag_total(b: Bag) -> u32 {
    b.items.iter().map(|i| i.n).sum()
}

/// Rule 1, via constructor: both of `Basket`'s own fields are private, so
/// the only way to build one is through `Basket::new`, whose own argument
/// is the same `Vec<Item>` shape as `Bag`'s field above -- the
/// constructor-argument counterpart of that field.
pub struct Basket {
    items: Vec<Item>,
}

impl Basket {
    pub fn new(items: Vec<Item>) -> Self {
        Basket { items }
    }
}

#[ply::ensures(|result| *result <= 10)]
pub fn basket_total(b: Basket) -> u32 {
    b.items.iter().map(|i| i.n).sum()
}

/// The third site: an enum variant's own field.
pub enum Holder {
    Full { items: Vec<Item> },
    Empty,
}

#[ply::ensures(|result| *result <= 10)]
pub fn holder_total(h: Holder) -> u32 {
    match h {
        Holder::Full { items } => items.iter().map(|i| i.n).sum(),
        Holder::Empty => 0,
    }
}
