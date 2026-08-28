//! Adversarial review, docs/review-caveats.md, "also fix": declaring
//! `[test, fuzz(n)]` together silently dropped the check that ran nothing,
//! undoing the guard added the same day (a pass must prove a case ran).
//! `test` alone runs a receiver method's `examples:`/direct-contract cases,
//! and this fn declares neither -- `test` contributes zero cases here. The
//! promise below is genuinely TRUE (`n` is clamped to at most 10 by `new`,
//! so `value()` never exceeds it) and `fuzz` genuinely passes: this is
//! deliberate, because a *false* promise would let the overall verdict's
//! worst-of aggregation report `violation` regardless of what `test` did,
//! hiding exactly the bug this fixture exists to catch. With a true
//! promise, before the fix, `fuzz`'s own tests running at all made the
//! *module-wide* test count nonzero, which let `test` fall through to
//! `tested` on zero cases of its own -- a check that ran nothing, reported
//! as a pass, beside a check that genuinely ran and also passed.

pub struct Gauge {
    n: u32,
}

impl Gauge {
    pub fn new(n: u32) -> Self {
        Gauge { n: n.min(10) }
    }

    /// TRUE on every input: `new` clamps `n` to at most 10.
    #[ply::ensures(|result| *result <= 10)]
    pub fn value(&self) -> u32 {
        self.n
    }
}
