//! Real-world reproduction (2026-09-01, verified by hand against `semver`'s
//! `Version::parse`): a factually false `examples:` entry passes in total
//! silence under `checks: [fuzz(64)]`. `fuzz` never compiles or runs a
//! declared example -- only `test` does -- and nothing said so, even though
//! the run *notices* the example changed and re-checks because of it (§5.2a
//! reads and fingerprints it as part of what this claim depends on). Ply
//! must now warn, naming the example that will never run and what to
//! declare instead, without changing the verdict itself.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_declared_example_no_check_will_run_is_disclosed_never_silent() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("examplesnotrun");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    // The false example must not change what `fuzz(64)` itself earned --
    // this is a warning, never a verdict change, because `fuzz` really did
    // check the true `#[ply::ensures]` contract against 64 real cases.
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the real contract holds and `fuzz(64)` ran for real -- the ignored, false example \
         must not be allowed to change this verdict, only to be disclosed: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["node_id"] == "examplesnotrun::increment" && d["code"] == "W0525")
        .unwrap_or_else(|| {
            panic!(
                "no W0525 diagnostic disclosing the never-run example -- a declared `examples:` \
                 entry that no check runs must never pass in total silence: {}",
                run.json
            )
        });

    assert_eq!(
        d["severity"], "warning",
        "this is a warning, not a verdict change: {d}"
    );

    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains('1'),
        "the warning must name how many examples will not run: {title}"
    );
    assert!(
        title.contains("not run"),
        "the warning must say plainly that the example was not run: {title}"
    );
    assert!(
        title.contains('`') && title.contains("test") && title.contains("checks:"),
        "the warning must say what to declare instead (`test` in `checks:`): {title}"
    );
}
