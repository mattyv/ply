//! M4 acceptance: a real, exact spec that `fuzz` + `test` together kill
//! every mutant of earns `·spec-strong` (D12: `test`/`fuzz` are `mutate`'s
//! kill signal; §1: "`fuzzed(n)·spec-strong` is the workhorse tier").

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn strong_spec_earns_the_spec_strong_suffix() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("strongspec");

    let run = run_verify(&cargo_ply, fixture.path(), 150);
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)\u{00b7}spec-strong",
        "envelope: {}",
        run.json
    );
    assert_eq!(
        run.json["diagnostics"].as_array().unwrap().len(),
        0,
        "envelope: {}",
        run.json
    );
    let statuses = run.json["root"]["children"][0]["children"][0]["statuses"]
        .as_array()
        .unwrap();
    assert!(
        statuses.is_empty(),
        "a strong spec must not carry weak-spec: {}",
        run.json
    );
}
