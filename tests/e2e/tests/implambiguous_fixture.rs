//! Two `impl` blocks for the same type, each defining a method of the same
//! name (real Rust: two concrete instantiations of a generic type). Ply must
//! diagnose the ambiguity rather than silently pick one -- picking wrong
//! would attach a verdict to the wrong function, which this project treats
//! as worse than reporting nothing (The-Ply-Spec.md §1, §5.2).

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn two_impl_blocks_with_a_same_named_method_produce_an_ambiguity_diagnostic_not_a_guess() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implambiguous");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(n["id"], "Wrapper::describe", "{}", run.json);
    assert_eq!(
        n["verdict"], "unsupported",
        "an ambiguous anchor must not silently pick one of the two candidates: {}",
        run.json
    );

    let d = run.json["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("no diagnostics: {}", run.json))
        .iter()
        .find(|d| d["node_id"] == "implambiguous::Wrapper::describe")
        .unwrap_or_else(|| panic!("no diagnostic for the ambiguous claim: {}", run.json));
    assert_ne!(
        d["code"], "E0301",
        "\"could not find\" is false -- Ply found two candidates, not zero: {d}"
    );
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("Wrapper::describe"),
        "the diagnostic must name the claim: {d}"
    );
    assert!(
        title.to_lowercase().contains("does not name one function")
            || title.to_lowercase().contains("ambiguous")
            || title.contains('2'),
        "the diagnostic must say plainly that more than one candidate matched: {d}"
    );
}
