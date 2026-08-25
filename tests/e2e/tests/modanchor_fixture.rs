//! A promise must be attachable to the function it describes, wherever that
//! function lives.
//!
//! Ply's two halves disagreed about where a function is. Call classification
//! (§5.5) follows `use` imports, inline `mod`s and file modules, so it could
//! see `rates::legacy_rate` well enough to refuse to descend into it. Anchor
//! resolution (§5.2) read `src/lib.rs` and walked its top-level items only,
//! so the claim that would have vouched for that same function was rejected
//! with `E0301`. Ply said "nobody has vouched for this" and then refused to
//! let anyone vouch for it.
//!
//! That closed off the whole per-function-promise route for real code, which
//! lives in `rates.rs` or `pricing.rs` or inside a `mod` and essentially
//! never at the top level of the crate root.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_promise_attaches_to_a_function_inside_a_file_module() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("modanchor");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "E0301"),
        "the claim points at `rates::legacy_rate`, which exists in `src/rates.rs` and which \
         Ply's own call classification already resolves: {}",
        run.json
    );

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert_eq!(
        verdict, "bounded(2)",
        "the promise describes the callee the caller crosses into, so the caller is provable \
         against it -- exactly as it is when the same callee sits at the top level of lib.rs: {}",
        run.json
    );

    let w0511 = diagnostics
        .iter()
        .find(|d| d["code"] == "W0511")
        .unwrap_or_else(|| panic!("no W0511 in {}", run.json));
    let title = w0511["title"].as_str().unwrap();
    assert!(
        title.contains("rates::legacy_rate"),
        "the assumption must name the callee the way the user wrote it: {title}"
    );
}

#[test]
fn the_module_fn_earns_no_node_of_its_own() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("modanchor");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    // §5.5: a fn entry that declares a contract and asks for no checks is a
    // boundary contract declaration, not a claim. Resolving it must not turn
    // it into an `unclaimed` node, which would say the opposite of what the
    // user wrote.
    let component = &run.json["root"]["children"][0];
    let ids: Vec<&str> = component["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["tiered_fee"], "envelope: {}", run.json);
}
