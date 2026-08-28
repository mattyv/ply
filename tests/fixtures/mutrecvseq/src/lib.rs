//! Adversarial review, docs/review-caveats.md N3: "the twelfth false clean".
//! An ordinary Rust type that mutates through `&mut self` -- the shape
//! *most* Rust types use, not an exotic one. Before the fix, Ply's bounded
//! operation sequence only ever pooled `&self` operations whose own
//! parameters matched the checked method's shape exactly, so `add` (taking
//! `&mut self`, one `u32` parameter) could never be spliced in before
//! `get` (taking `&self`, no parameters at all) -- every generated case
//! called `get` on a freshly constructed, never-mutated `Acc`, and the
//! promise below, false after two ordinary calls (`a.add(3); a.add(3);
//! a.get() == 6`), reported a clean `fuzzed(256)` forever.

pub struct Acc {
    n: u32,
}

impl Acc {
    pub fn new() -> Self {
        Acc { n: 0 }
    }

    /// The ordinary way this type changes state: `&mut self`, and its own
    /// parameter shape differs from `get`'s (one `u32` vs none) -- both
    /// reasons the old pool excluded it.
    pub fn add(&mut self, k: u32) -> u32 {
        self.n += k;
        self.n
    }

    /// FALSE after any two calls to `add` whose total exceeds 4: a fresh
    /// `Acc` always holds 0, so a receiver built from the constructor alone
    /// (or from repeating `get` itself, which changes nothing) can never
    /// see this fail.
    #[ply::ensures(|result| *result < 5)]
    pub fn get(&self) -> u32 {
        self.n
    }
}
