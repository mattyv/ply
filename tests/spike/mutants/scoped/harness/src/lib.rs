//! Stand-in for a Ply-generated harness crate (§5.4c row: "generated
//! harness crate under `target/ply/fuzz/`"). Tests are namespaced per
//! target function (`strong_target_harness::*` / `weak_target_harness::*`)
//! so a cargo-test name filter can select exactly one function's checks,
//! which is the mechanism the item-3 experiment is proving out.

#[cfg(test)]
mod strong_target_harness {
    use ply_spike_mutants_lib::strong_target;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn fuzz_strong_target(
            x in -1_000_000_i32..1_000_000,
            y in -1_000_000_i32..1_000_000,
        ) {
            let got = strong_target(x, y);
            let expected = if x > 0 && y > 0 { x + y } else { x - y };
            prop_assert_eq!(got, expected);
        }
    }

    #[test]
    fn example_strong_target_boundaries() {
        assert_eq!(strong_target(0, 5), -5);
        assert_eq!(strong_target(5, 0), 5);
        assert_eq!(strong_target(1, 1), 2);
    }
}

#[cfg(test)]
mod weak_target_harness {
    use ply_spike_mutants_lib::weak_target;

    #[test]
    fn example_weak_target_smoke() {
        // Vacuous, deliberately: only checks the call returns.
        let _ = weak_target(3, 4);
    }
}
