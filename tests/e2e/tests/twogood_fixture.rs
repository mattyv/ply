//! §9: "green is not a merge argument", but the reverse matters just as
//! much for this fix -- a crate where every claim is fine must behave
//! exactly as before. The misattribution fix adds a preflight build check
//! ahead of every fuzz/test claim's own harness run; this fixture is the
//! guard against that check itself introducing noise, an extra
//! diagnostic, or a changed verdict on a crate with nothing wrong in it.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_crate_with_two_correct_fuzz_test_claims_reports_exactly_as_before() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("twogood");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(32)",
        "envelope: {}",
        run.json
    );
    assert_eq!(
        run.json["diagnostics"].as_array().unwrap().len(),
        0,
        "two completely correct claims must carry no diagnostics at all: {}",
        run.json
    );
    assert_eq!(run.exit_code, Some(0), "a clean verify must exit 0");

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    assert_eq!(children.len(), 2, "envelope: {}", run.json);
    for child in children {
        assert!(
            child["verdict"].as_str().unwrap().starts_with("fuzzed")
                || child["verdict"] == "tested",
            "every claim in a clean crate must earn real evidence, not `unclaimed` or an \
             absence: {child}"
        );
        assert!(
            child["evidence"].is_object() || child["verdict"] == "tested",
            "a fuzz verdict must name the run that produced it: {child}"
        );
    }
}
