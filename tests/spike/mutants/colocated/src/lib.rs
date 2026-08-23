//! cargo-mutants spike fixture (§10 M0, §5.4c / D12).
//!
//! Two functions, identical bodies, deliberately full of obviously-plantable
//! mutants (a comparison, a boolean `&&`, and a `+`/`-` choice): one is
//! backed by a real property test standing in for a Ply-generated `fuzz`
//! harness (D12's `mutate` kill signal), the other by a vacuous test that
//! only checks the call doesn't panic — standing in for an underspecified
//! `test`/`fuzz` list. `cargo mutants` should catch mutants in the former
//! and let mutants in the latter survive; that gap is what makes `mutate`
//! meaningful as a weak-spec detector (D12).

/// Strong spec: pinned tightly by `strong_target_matches_reference` below.
pub fn strong_target(x: i32, y: i32) -> i32 {
    if x > 0 && y > 0 {
        x + y
    } else {
        x - y
    }
}

/// Weak spec: same body, but its own test constrains almost nothing.
pub fn weak_target(x: i32, y: i32) -> i32 {
    if x > 0 && y > 0 {
        x + y
    } else {
        x - y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Strong spec: pins strong_target's exact output for every input
        /// pair (bounded away from overflow so the property, not a panic,
        /// is what's under test) against an independently-written
        /// reference computation, and pins the algebraic shape of the
        /// branch condition too.
        #[test]
        fn strong_target_matches_reference(
            x in -1_000_000_i32..1_000_000,
            y in -1_000_000_i32..1_000_000,
        ) {
            let got = strong_target(x, y);
            let expected = if x > 0 && y > 0 { x + y } else { x - y };
            prop_assert_eq!(got, expected);

            // Independent corroborating properties, so a mutant that
            // happens to satisfy the equality above by coincidence on the
            // sampled cases still has more surface to be caught on.
            if x > 0 && y > 0 {
                prop_assert!(got > x && got > y);
            } else {
                prop_assert_eq!(got, x - y);
            }
        }
    }

    /// Explicit boundary examples, standing in for D12's `test` entry
    /// alongside `fuzz` in the same `checks` list: uniform random sampling
    /// over a wide range essentially never lands exactly on x=0/y=0, so
    /// `strong_target_matches_reference` alone left a `>` vs `>=` mutant
    /// at the boundary surviving (observed: cargo-mutants MISSED both
    /// `replace > with >=` mutants until these were added). `test` and
    /// `fuzz` together are the kill signal D12 names — not fuzz alone.
    #[test]
    fn strong_target_boundary_cases() {
        assert_eq!(strong_target(0, 5), -5);
        assert_eq!(strong_target(5, 0), 5);
        assert_eq!(strong_target(1, 1), 2);
        assert_eq!(strong_target(0, 0), 0);
        assert_eq!(strong_target(-1, -1), 0);
    }

    #[test]
    fn weak_target_does_not_panic() {
        // Vacuous: exercises one call and asserts nothing about the
        // result. Stands in for a `checks: [fuzz(1)]` list too thin to
        // constrain behaviour — the case D12/W0502 exists to flag.
        let _ = weak_target(3, 4);
    }
}
