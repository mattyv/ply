//! The exact repro that found both 2026-08-26 defects.
//!
//! Before the fix: `vector`'s postcondition read `v.len()` after `v` had
//! already been moved into the call, so its generated harness failed to
//! compile with `error[E0382]: borrow of moved value: `v``; and because
//! `scalar` shared the same generated harness crate (§5.4c), `scalar` --
//! which takes no `v` at all -- came back `tool_error` quoting that same
//! error too.
//!
//! Defect B's fix refuses `vector`'s contract by name instead of ever
//! generating the broken code, so this fixture never actually exercises a
//! compile failure any more (see `sharedharness_fixture.rs` for that shape,
//! via a broken `examples:` entry instead). What it does confirm: the
//! refusal is by name, and `scalar` earns its own real verdict regardless.

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
fn a_postcondition_reading_a_moved_parameter_is_refused_and_its_crate_mate_is_unaffected() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("movedparam");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    let diagnostics = run.json["diagnostics"].as_array().unwrap();

    // No run may end in an internal error about Ply's own generated code --
    // that is not an answer about the user's function.
    assert!(
        diagnostics.iter().all(|d| d["code"] != "X0901"),
        "the moved-parameter shape must be refused before it ever reaches codegen, not \
         discovered as a compile failure: {}",
        run.json
    );

    // Refused by name, not silently dropped and not miscategorized.
    let vector = node(&run.json, "vector");
    assert_eq!(vector["verdict"], "unsupported", "envelope: {}", run.json);
    let refusal = diagnostics
        .iter()
        .find(|d| d["node_id"] == "movedparam::vector")
        .unwrap_or_else(|| panic!("no diagnostic for vector: {}", run.json));
    assert_eq!(refusal["code"], "V0506");
    let title = refusal["title"].as_str().unwrap();
    assert!(title.contains("`v`"), "must name the parameter: {title}");
    assert!(
        title.contains("moved"),
        "must say what actually goes wrong, not just that it failed: {title}"
    );
    assert!(
        title.contains("Vec<u8>") || title.contains("Vec < u8 >"),
        "must name the parameter's type: {title}"
    );

    // Unrelated, and correct -- must earn its own real verdict.
    let scalar = node(&run.json, "scalar");
    assert_eq!(
        scalar["verdict"], "fuzzed(32)",
        "scalar takes no `v` at all and is completely correct: {}",
        run.json
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "movedparam::scalar"),
        "scalar must carry no diagnostic of its own: {}",
        run.json
    );
}
