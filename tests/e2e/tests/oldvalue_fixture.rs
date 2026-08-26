//! §5.4a's `old(...)` on the fuzz/test path, and the one shape it cannot
//! reach, both run end to end (2026-08-25).
//!
//! Before this fixture existed, `cargo ply verify` on both functions
//! answered with an internal error about Ply rather than an answer about
//! the code:
//!
//! ```text
//! [X0901] oldvalue::bump — `bump`'s `fuzz(64)` check ran zero cases: the
//! test harness Ply generates for it failed to compile ... The compiler's
//! own first error was: error[E0425]: cannot find function `old` in this
//! scope.
//!
//! [X0901] oldvalue::bump_in_place — `bump_in_place`'s `fuzz(64)` check ran
//! zero cases ... error[E0308]: mismatched types.
//! ```

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    fn find<'a>(n: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
        if n["id"] == id {
            return Some(n);
        }
        n["children"]
            .as_array()?
            .iter()
            .find_map(|child| find(child, id))
    }
    find(&json["root"], id).unwrap_or_else(|| panic!("no node `{id}` in envelope: {json}"))
}

#[test]
fn an_entry_value_is_checked_for_real_and_a_written_back_parameter_is_refused_by_name() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("oldvalue");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().all(|d| d["code"] != "X0901"),
        "no run may end in an internal error about Ply's own generated code -- that is not an \
         answer about the user's function: {}",
        run.json
    );

    // The construct works: a contract that refers to a parameter's value on
    // entry earns real evidence from generated inputs.
    assert_eq!(
        node(&run.json, "bump")["verdict"],
        "fuzzed(64)",
        "envelope: {}",
        run.json
    );

    // The shape it exists for is refused, and the refusal names the type
    // the user wrote.
    assert_eq!(
        node(&run.json, "bump_in_place")["verdict"],
        "unsupported",
        "envelope: {}",
        run.json
    );
    let refusal = diagnostics
        .iter()
        .find(|d| d["node_id"] == "oldvalue::bump_in_place")
        .unwrap_or_else(|| panic!("no diagnostic for bump_in_place: {}", run.json));
    assert_eq!(refusal["code"], "V0505");
    let title = refusal["title"].as_str().unwrap();
    assert!(
        title.contains("counter: &mut u32"),
        "the refusal must name the parameter and spell its type the way the user wrote it: \
         {title}"
    );

    // The generated harness reads the entry value before the call, which is
    // the whole meaning of the construct.
    let harness_dir = fixture.path().join("target/ply/fuzz");
    assert!(harness_dir.is_dir(), "expected a generated harness crate");
    let harness = std::fs::read_dir(&harness_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("src/lib.rs"))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("no generated harness source under {harness_dir:?}"));
    let source = std::fs::read_to_string(&harness).unwrap();
    let snapshot = source
        .find("let __ply_old_0")
        .unwrap_or_else(|| panic!("entry value is never read into a binding:\n{source}"));
    let call = source
        .find("let __ply_call_result")
        .expect("generated harness always binds the call result");
    assert!(snapshot < call, "read the entry value first:\n{source}");
}
