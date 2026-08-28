//! The eleventh false pass, "one step milder" (task 2026-08-27,
//! docs/review-strings-receivers.md finding 1): a function with no
//! `#[ply::ensures]` and no `examples:`, declared with only `test`. There is
//! nothing for `test` to assert, so its harness module used to contribute
//! no test, `cargo test`'s own filter matched zero tests, and the run used
//! to report `tested`/held with zero cases run -- the same mechanism as the
//! receiver-method false pass, on a plain free function this time. Pinned
//! separately from `falsereceivertest` because the generic "zero tests
//! executed" guard must catch every shape that reaches it, not only a
//! receiver method.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_test_check_with_no_contract_and_no_examples_is_never_a_clean_pass() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nopromisetest");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert_ne!(
        verdict, "tested",
        "a `test` check with nothing to assert must never read as a held promise: {}",
        run.json
    );
    assert_eq!(verdict, "unsupported", "{}", run.json);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["node_id"] == "nopromisetest::seven" && d["code"] == "V0505")
        .unwrap_or_else(|| panic!("no V0505 diagnostic: {}", run.json));
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("no `#[ply::ensures]`") && title.contains("no `examples:`"),
        "the diagnostic must name exactly why nothing ran: {title}"
    );
}
