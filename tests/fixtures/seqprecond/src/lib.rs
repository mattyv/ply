//! Adversarial review, docs/review-caveats.md N2: "the operations run before
//! the checked call... ignore the checked method's own precondition." The
//! bounded sequence Ply builds always includes the checked method itself as
//! one of its pooled operations (repeating it is what reaches a second-call
//! bug -- see `tests/fixtures/receiverseq`), but before this fix only the
//! *final* call's arguments were filtered by `#[ply::requires]`; every
//! earlier repeat inside the sequence drew its own arguments unfiltered.
//! `Thing::set`'s own postcondition (`result <= 10`) is true of every value
//! its own precondition (`k <= 10`) admits -- it can never fail on a call
//! that honours the contract. The only way Ply ever reported a violation
//! here was by calling `set(11)` as an *earlier* step in the sequence, a
//! call `set`'s own contract forbids and that panics on entry -- and that
//! panic was attributed to `set` "failing its own contract", when the real
//! defect was Ply calling it outside its own precondition.

pub struct Thing {
    n: u32,
}

impl Thing {
    pub fn new() -> Self {
        Thing { n: 0 }
    }

    /// `result <= 10` holds for every `k` the precondition admits -- this
    /// can only ever be found broken by a call `set`'s own contract
    /// forbids.
    #[ply::requires(k <= 10)]
    #[ply::ensures(|result| *result <= 10)]
    pub fn set(&self, k: u32) -> u32 {
        assert!(k <= 10, "set's own precondition is k <= 10");
        k
    }
}
