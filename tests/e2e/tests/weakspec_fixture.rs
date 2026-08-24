//! M4 acceptance: a vacuous `ensures` earns `W0502` weak spec -- `mutate`
//! must find every mutant surviving a postcondition that can never fail.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn vacuous_ensures_is_flagged_weak_spec() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("weakspec");

    let run = run_verify(&cargo_ply, fixture.path(), 150);
    // The base fuzz check itself still passes (the vacuous `ensures` never
    // fails) -- `mutate` is the one that must catch the weakness, as a
    // status flag alongside the verdict (D6: statuses propagate alongside
    // the evidence order, they don't replace it).
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "envelope: {}",
        run.json
    );
    let statuses = run.json["root"]["children"][0]["children"][0]["statuses"]
        .as_array()
        .unwrap();
    assert!(
        statuses.iter().any(|s| s == "weak-spec"),
        "the fn node must carry a weak-spec status: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "W0502");
    assert_eq!(diag["engine"], "cargo-mutants");
    let title = diag["title"].as_str().unwrap();
    assert!(title.contains("weak spec"), "{title}");
    assert!(title.contains("2 surviving mutants"), "{title}");
    // §5.4c's own equivalent-mutant caveat, carried into the wording so a
    // reader chasing "N surviving mutants" to zero does not burn time on
    // mutants nothing could ever catch.
    assert!(
        title.contains("equivalent mutant") || title.contains("not every entry"),
        "W0502 must carry the equivalent-mutant caveat: {title}"
    );
}
