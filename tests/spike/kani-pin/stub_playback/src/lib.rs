//! Blocker 1: does `#[kani::stub]` compose with `--concrete-playback`?
//!
//! At the pinned 0.67.0 the stubbing book says, under a heading that claims to
//! list *all* the limitations: "this feature isn't compatible with concrete
//! playback". Kani `main` deletes that sentence and says the opposite. §8 of
//! The-Ply-Spec.md forbids emitting a `violation` without a witness, so if a
//! stubbed harness can fail without producing one, §5.5's boundary rule can
//! never report a violation at all.
//!
//! Two harnesses, deliberately different in *where* the failure comes from:
//!
//!   * `check_stub_only_failure` fails only because of the value the stub
//!     invented. A witness for it must record the stub's `kani::any()`.
//!   * `check_input_failure_with_stub` carries the same stub but fails on the
//!     harness's own input, which playback is already known to capture.
//!
//! The second is the control: if it replays red and the first replays green,
//! playback is running but is blind to stubbed values.

/// The real callee. Concrete, and the whole point: the stub replaces it with
/// something strictly less informative.
pub fn rate() -> u32 {
    7
}

/// The caller under proof. `wrapping_mul` so the only failure available is
/// the assertion, never an arithmetic-overflow check.
pub fn total(n: u32) -> u32 {
    n.wrapping_mul(rate())
}

#[cfg(kani)]
#[allow(dead_code)]
mod proofs {
    use super::*;

    /// Stands in for `rate`, returning any value up to 10 instead of exactly 7.
    fn stub_rate() -> u32 {
        let r: u32 = kani::any();
        kani::assume(r <= 10);
        r
    }

    /// FAILS -- and only because of the stub. With the real `rate` (== 7) and
    /// `n <= 3`, `total(n) <= 21` holds. With the stub (`rate <= 10`) it does
    /// not: n = 3, rate = 10 gives 30.
    #[kani::proof]
    #[kani::stub(rate, stub_rate)]
    fn check_stub_only_failure() {
        let n: u32 = kani::any();
        kani::assume(n <= 3);
        assert!(total(n) <= 21, "total(n) exceeded 21");
    }

    /// FAILS on the harness's own input, with the same stub attached. The
    /// control for the harness above.
    #[kani::proof]
    #[kani::stub(rate, stub_rate)]
    fn check_input_failure_with_stub() {
        let n: u32 = kani::any();
        kani::assume(n <= 3);
        let _ = total(n);
        assert!(n != 3, "n reached 3");
    }
}
