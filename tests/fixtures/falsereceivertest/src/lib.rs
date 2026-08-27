//! The eleventh false pass, pinned as a permanent fixture
//! (docs/review-strings-receivers.md finding 1, task 2026-08-27): a receiver
//! method with a promise that is false on *every* input, declared with only
//! the `test` check and no worked examples. `Calc::value` never generates a
//! worked-examples/direct-contract test body at all (no `examples:` entries,
//! and a receiver method's concrete direct-contract case is deliberately
//! left to the sampling tier -- see `fuzz_gen::generate_direct_contract_cases`'s
//! own doc), so its harness module used to contribute nothing, `cargo test`'s
//! own filter matched zero tests, the run exited 0, and "no failing test" was
//! read as `tested`/held. The bug: `v` is clamped to at most 10 by the
//! constructor, so `value()` can never reach 1000.

pub struct Calc {
    pub v: u32,
}

impl Calc {
    pub fn new(v: u32) -> Self {
        Self { v: v.min(10) }
    }

    /// FALSE on every input: `v` is at most 10 (see `new`), so `value()`
    /// never reaches 1000.
    #[ply::ensures(|result| *result >= 1000)]
    pub fn value(&self) -> u32 {
        self.v
    }
}
