//! docs/review-silent-narrowing.md finding 1, 2026-08-28: `Till`'s only
//! mutating operation (`take`) lives in a second, ordinary `impl Till`
//! block in a different file than the one `Till` and the checked method
//! (`total`) are declared in. Before this fixture's fix, the receiver scan
//! read the declaring file only, so `take` never entered the operation
//! pool and `total`'s promise (always 0, false after one real `take` call)
//! reported a clean, unqualified `fuzzed(n)` pass forever.
//!
//! Unlike the trait-method and second-constructor cases (their own
//! fixtures), this one is genuinely fixable rather than merely
//! disclosable: a second file's `impl` block is ordinary Rust the scan can
//! and now does open. So the acceptance bar here is stronger than "named
//! honestly" -- the run must actually FIND the violation.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_mutator_in_a_second_file_is_found_and_the_false_promise_is_caught() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("crossfilemutator");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // The whole point: this is not a disclosure case, it is a caught bug.
    // A pool that still missed `take` would report `fuzzed(256)` here,
    // which is exactly the false clean this fixture exists to end.
    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "`till::Till::take` lives in a second file (`more.rs`) via an ordinary second `impl \
         Till` block -- the receiver scan must open every file under `src/`, find it, call it, \
         and catch `total`'s false promise: {}",
        run.json
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "a real violation must fail the run: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let sequence_disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "crossfilemutator::till::Till::total" && d["code"] == "W0520")
        .unwrap_or_else(|| panic!("no W0520 sequence disclosure: {}", run.json));
    let title = sequence_disclosure["title"].as_str().unwrap();
    assert!(
        title.contains("Till::take"),
        "the pool must have actually included the cross-file mutator, not merely found it and \
         excluded it: {title}"
    );
}
