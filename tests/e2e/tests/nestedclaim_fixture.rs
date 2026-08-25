//! A claim written inside a nested component must be checked, and must be
//! reported (The-Ply-Spec.md §5.1's nested `components:`, §6's one verdict
//! per claim).
//!
//! `verify` iterated `document.components` and each component's `fns`, and
//! stopped there: a claim one level down produced no node, no diagnostic and
//! no mention in either surface, while `cargo ply check` walked the whole
//! tree and reported the very same claim as pointing at real code. The
//! document validated, the claim never ran, and nothing said so.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_claim_inside_a_nested_component_earns_a_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nestedclaim");

    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let outer = &run.json["root"]["children"][0];
    assert_eq!(outer["id"], "outer", "envelope: {}", run.json);
    let inner = &outer["children"][0];
    assert_eq!(
        inner["id"], "outer.inner",
        "a nested component is a node under its parent, named the way `check` names it \
         (`outer.inner`): {}",
        run.json
    );
    let fn_node = &inner["children"][0];
    assert_eq!(
        fn_node["id"], "safe_increment",
        "the claim itself must be in the tree: {}",
        run.json
    );
    assert_eq!(
        fn_node["verdict"], "bounded(2)",
        "the claim asks for a proof and the contract holds, so it earns one -- a claim that \
         is skipped for being nested is a claim nobody checked: {}",
        run.json
    );
    assert_eq!(
        run.json["root"]["verdict"], "bounded(2)",
        "and the verdict travels to the root: {}",
        run.json
    );
    assert_eq!(run.exit_code, Some(0), "envelope: {}", run.json);
}

/// The human surface is the one most people read, and it is where the claim
/// went missing without a trace.
#[test]
fn the_nested_claim_is_named_in_the_terminal_output() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nestedclaim");

    let out = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap()])
        .arg("--engine-timeout")
        .arg("90")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(
        stdout.contains(
            "workspace — bounded(2)\n  outer — bounded(2)\n    outer.inner — bounded(2)\n      \
             safe_increment — bounded(2)\n"
        ),
        "the printed tree must show the nested component and the claim inside it: {stdout}"
    );
}
