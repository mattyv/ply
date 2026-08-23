//! ADR-0003 feasibility spike fixture. Throwaway crate — see tests/spike/FINDINGS.md.
//! Kani pinned: cargo-kani 0.67.0 (CBMC 6.8.0).

// ---------------------------------------------------------------------
// Item 1: private free function, contracted, proved via proof_for_contract.
// Harness lives in an in-crate `#[cfg(kani)]` module (also exercises item 9).
// ---------------------------------------------------------------------

#[cfg_attr(kani, kani::requires(x < 1000))]
#[cfg_attr(kani, kani::ensures(|result| *result == x + 1))]
fn private_increment(x: u32) -> u32 {
    x + 1
}

#[cfg(kani)]
mod item1_proofs {
    use super::*;

    #[kani::proof_for_contract(private_increment)]
    fn check_private_increment() {
        let x: u32 = kani::any();
        private_increment(x);
    }
}

// ---------------------------------------------------------------------
// Item 2: contracted method on an impl block, &self receiver.
// ---------------------------------------------------------------------

pub struct Counter {
    value: u32,
}

impl Counter {
    #[cfg_attr(kani, kani::requires(self.value < 1000))]
    #[cfg_attr(kani, kani::ensures(|result| *result == old(self.value) + 1))]
    pub fn bump(&self) -> u32 {
        self.value + 1
    }
}

#[cfg(kani)]
mod item2_proofs {
    use super::*;

    #[kani::proof_for_contract(Counter::bump)]
    fn check_counter_bump() {
        let value: u32 = kani::any();
        let counter = Counter { value };
        counter.bump();
    }
}

// ---------------------------------------------------------------------
// Item 3a: function taking a user-defined struct with public,
// invariant-free fields. kani::Arbitrary derive should make this trivial.
// ---------------------------------------------------------------------

#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[cfg_attr(kani, kani::requires(p.x < i32::MAX && p.y < i32::MAX))]
#[cfg_attr(kani, kani::ensures(|result| result.x == p.x + 1 && result.y == p.y + 1))]
pub fn shift_point(p: Point) -> Point {
    Point { x: p.x + 1, y: p.y + 1 }
}

#[cfg(kani)]
mod item3a_proofs {
    use super::*;

    #[kani::proof_for_contract(shift_point)]
    fn check_shift_point() {
        let p: Point = kani::any();
        shift_point(p);
    }
}

// ---------------------------------------------------------------------
// Item 3b: same shape, but with a private field carrying an invariant
// (value must stay non-zero). Documents what a private-field type needs:
// kani::Arbitrary can't be derived over a private field from outside the
// module, and #[derive] itself can't enforce the invariant — so either
// the type needs a hand-written `Arbitrary` impl that only ever
// constructs valid values, or a public smart constructor plus a
// `kani::any_where` filter in the harness.
// ---------------------------------------------------------------------

pub struct NonZero(i32);

impl NonZero {
    pub fn new(v: i32) -> Option<Self> {
        if v == 0 {
            None
        } else {
            Some(NonZero(v))
        }
    }

    pub fn get(&self) -> i32 {
        self.0
    }
}

#[cfg(kani)]
impl kani::Arbitrary for NonZero {
    fn any() -> Self {
        // Hand-written: derive can't see the private field from here if it
        // lived in a different module, and derive can't express "nonzero"
        // regardless. This impl is only possible because the harness lives
        // in the same crate as the private field (in-crate proof module,
        // item 9) -- a sibling crate could not write this at all.
        let inner: i32 = kani::any();
        kani::assume(inner != 0);
        NonZero(inner)
    }
}

#[cfg_attr(kani, kani::requires(n.get() != i32::MAX))]
#[cfg_attr(kani, kani::ensures(|result| result.get() == n.get() + 1))]
pub fn bump_nonzero(n: NonZero) -> NonZero {
    NonZero(n.0 + 1)
}

#[cfg(kani)]
mod item3b_proofs {
    use super::*;

    #[kani::proof_for_contract(bump_nonzero)]
    fn check_bump_nonzero() {
        let n: NonZero = kani::any();
        bump_nonzero(n);
    }
}

// ---------------------------------------------------------------------
// Item 4: same-crate stub_verified. `g` is proved by its own contract;
// `f` is verified with `g` stubbed by its contract instead of its body.
// This is the mechanism D5's modular-verification soundness rests on.
// ---------------------------------------------------------------------

#[cfg_attr(kani, kani::requires(x < 1000))]
#[cfg_attr(kani, kani::ensures(|result| *result == x + 1))]
fn g(x: u32) -> u32 {
    x + 1
}

#[cfg_attr(kani, kani::requires(x < 999))]
#[cfg_attr(kani, kani::ensures(|result| *result == x + 2))]
fn f(x: u32) -> u32 {
    g(g(x))
}

#[cfg(kani)]
mod item4_proofs {
    use super::*;

    #[kani::proof_for_contract(g)]
    fn check_g() {
        let x: u32 = kani::any();
        g(x);
    }

    #[kani::proof_for_contract(f)]
    #[kani::stub_verified(g)]
    fn check_f_stubs_g() {
        let x: u32 = kani::any();
        f(x);
    }
}

// ---------------------------------------------------------------------
// Item 5: cross-crate callee. Spec EXPECTS this to fail (D2's fallback
// note: "verifying pub items only from a sibling harness crate"). g_remote
// lives in ply-spike-fixture-callee; f_remote here calls it and tries to
// stub it via stub_verified across the crate boundary.
// ---------------------------------------------------------------------

#[cfg_attr(kani, kani::requires(x < 999))]
#[cfg_attr(kani, kani::ensures(|result| *result == x + 2))]
fn f_remote(x: u32) -> u32 {
    ply_spike_fixture_callee::g_remote(ply_spike_fixture_callee::g_remote(x))
}

#[cfg(kani)]
mod item5_proofs {
    use super::*;

    // Workaround attempt: declare a LOCAL proof_for_contract harness that
    // targets the remote function by qualified path, to see whether Kani's
    // existence check is satisfied by a harness anywhere in the crate being
    // compiled (rather than requiring the harness to live in the callee's
    // own crate).
    #[kani::proof_for_contract(ply_spike_fixture_callee::g_remote)]
    fn check_g_remote_locally() {
        let x: u32 = kani::any();
        ply_spike_fixture_callee::g_remote(x);
    }

    #[kani::proof_for_contract(f_remote)]
    #[kani::stub_verified(ply_spike_fixture_callee::g_remote)]
    fn check_f_remote_stubs_g_remote() {
        let x: u32 = kani::any();
        f_remote(x);
    }
}

// ---------------------------------------------------------------------
// Item 6: a real contract violation. `saturating_bump`'s ensures claims
// exact +1 growth, but the function actually saturates at u8::MAX -- the
// ensures is genuinely false for x == 255, and only there (for every
// x < 255, saturating_add(1) == x + 1 exactly). The violation manifests
// as an overflow panic *inside the ensures closure* (`x + 1` as u8),
// not as a failed comparison -- see item 7's commentary.
// ---------------------------------------------------------------------

#[cfg_attr(kani, kani::ensures(|result| *result == x + 1))]
pub fn saturating_bump(x: u8) -> u8 {
    x.saturating_add(1)
}

#[cfg(kani)]
mod item6_proofs {
    use super::*;

    #[kani::proof_for_contract(saturating_bump)]
    fn check_saturating_bump() {
        let x: u8 = kani::any();
        saturating_bump(x);
    }

    /// Test generated for harness `item6_proofs::check_saturating_bump` that checks contract for `saturating_bump`
    ///
    /// Check for `assertion`: "attempt to add with overflow"

    #[test]
    fn kani_concrete_playback_check_saturating_bump_5881385579587027251() {
        let concrete_vals: Vec<Vec<u8>> = vec![
        // 255
        vec![255],
    ];
    kani::concrete_playback_run(concrete_vals, check_saturating_bump);
}
}

// ---------------------------------------------------------------------
// Item 7: the same counterexample (x = 255), by hand, as a plain #[test]
// a human could read with no Kani knowledge. Written manually -- not
// generated -- to see what it costs to translate a cex into a test a
// reviewer could understand on sight.
//
// The raw Kani failure was "attempt to add with overflow" *inside the
// ensures closure* (`x + 1` overflows u8 for x = 255, so the check
// panics before it can even compare). That's a real detail worth
// preserving, but a test built only to reproduce the panic wouldn't
// show a Ply user *why the contract is wrong* -- it would just show
// that arithmetic overflows, which reads as a Rust footgun, not a
// contract bug. So this test widens to u16 to state the actual claim
// the contract is making (exact +1 growth) and show it's false at the
// boundary, which is the fact a human actually needs.
// ---------------------------------------------------------------------

#[cfg(test)]
mod item7_handwritten_test {
    use super::*;

    #[test]
    fn saturating_bump_breaks_its_own_contract_at_255() {
        let x: u8 = 255;
        let result = saturating_bump(x);

        // The contract claims: result == x + 1 (exact +1 growth, no cap).
        // Widen before comparing so the test can state the discrepancy
        // instead of reproducing the overflow panic Kani hit.
        let claimed = x as u16 + 1; // 256 -- not representable as u8
        let actual = result as u16; // 255 -- saturating_add clamped it

        assert_ne!(
            actual, claimed,
            "contract says saturating_bump({x}) == {claimed}, but it actually \
             returns {actual} because saturating_add clamps at u8::MAX; the \
             ensures clause is false at this input"
        );
    }
}
