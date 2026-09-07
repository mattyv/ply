//! A rule about a structure has to be checked by sequences long enough to
//! break it.
//!
//! `state.len() <= state.capacity()` is the plainest invariant Ply supports,
//! and until 2026-09-07 an off-by-one in the fullness test came back clean
//! over 256 cases: the generated call sequences ran at most three
//! operations while the capacity was drawn from 0..=16, so nearly every
//! generated cache was too large for any sequence to fill. Replaying the
//! generated strategy over twelve seeds, **2 of 3,072 cases** could reach
//! the bug at all -- five runs in six came back clean, and the sixth was
//! luck.
//!
//! Found in an A/B round by an agent that declared exactly the right rule,
//! broke its own eviction test to see whether Ply would notice, and
//! reported that it did not.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// Let the cache hold one entry too many, and the structural rule must say
/// so. Nothing else in this fixture can: there are no function claims.
#[test]
fn a_cache_that_overfills_by_one_is_caught_by_its_own_structural_rule() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("boundedcache");

    let src = fixture.read_lib_rs();
    // `>` instead of `>=`: a cache already holding `capacity` entries is not
    // treated as full, so the push takes it to `capacity + 1`.
    let broken = src.replace(
        "if self.entries.len() >= self.capacity {",
        "if self.entries.len() > self.capacity {",
    );
    assert_ne!(src, broken, "the fullness test must have been rewritten");
    fixture.write_lib_rs(&broken);

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert_eq!(
        verdict, "violation",
        "the cache can now hold more entries than its capacity, which is the one rule \
         this component declares, so the run has to report a violation: {}",
        run.json
    );
}
