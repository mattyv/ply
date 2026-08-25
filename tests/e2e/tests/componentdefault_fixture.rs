//! A component's default `checks:` governs the functions inside it
//! (The-Ply-Spec.md §5.1), in the command that runs the checks as well as
//! in the one that validates the document.
//!
//! `verify` read a function's own list and, finding none, fell through to
//! the shape-aware default -- so a component that declared `fuzz(64)` for
//! everything under it got model-checking proofs instead, and nothing said
//! the declared default had been passed over. `check` resolved the same
//! line correctly. One document, two answers.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_component_default_is_what_actually_runs() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("componentdefault");

    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let outer = &run.json["root"]["children"][0];
    let verdict = |node: &serde_json::Value, id: &str| -> String {
        node["children"]
            .as_array()
            .unwrap_or_else(|| panic!("no children under {node}"))
            .iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("no node `{id}`: {}", run.json))["verdict"]
            .as_str()
            .unwrap()
            .to_string()
    };

    assert_eq!(
        verdict(outer, "takes_the_default"),
        "fuzzed(64)",
        "the component asked for `fuzz(64)` and that is what must have run -- a proof here \
         means the declared default was ignored: {}",
        run.json
    );
    assert_eq!(
        verdict(outer, "writes_its_own"),
        "bounded(2)",
        "a fn's own list still wins entirely over the default above it: {}",
        run.json
    );

    let inner = outer["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "outer.inner")
        .unwrap_or_else(|| panic!("no nested component node: {}", run.json));
    assert_eq!(
        verdict(inner, "nested_takes_the_default"),
        "fuzzed(64)",
        "a nested component that declares no default of its own inherits the one above it: {}",
        run.json
    );

    assert_eq!(run.exit_code, Some(0), "envelope: {}", run.json);
}
