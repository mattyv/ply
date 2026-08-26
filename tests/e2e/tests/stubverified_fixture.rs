//! D5's first branch (The-Ply-Spec.md §5.5): a same-crate callee that this
//! run proved `bounded(k)` on its own is stood on, not merely assumed. The
//! caller must come back clean -- not `conditional`, not carrying owed
//! evidence for that callee -- while the tree still shows the dependency
//! (an `info`-severity `W0517`, never a warning: nothing here is owed).
//!
//! Before this branch existed, every same-crate contracted callee simply
//! let Kani inline its real body (no stub of any kind), which is why a
//! trivial two-function call chain like this one could time out at the
//! tool's own default budget rather than reuse the callee's own proof --
//! see this file's own red-first run in the session that added it.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_callee_proved_this_run_is_stood_on_not_merely_assumed() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverified");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let fn_nodes = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let f = fn_nodes
        .iter()
        .find(|n| n["id"] == "f")
        .unwrap_or_else(|| panic!("no `f` node in envelope: {}", run.json));
    let g = fn_nodes
        .iter()
        .find(|n| n["id"] == "g")
        .unwrap_or_else(|| panic!("no `g` node in envelope: {}", run.json));

    assert_eq!(g["verdict"], "bounded(2)", "envelope: {}", run.json);
    assert_eq!(
        f["verdict"], "bounded(2)",
        "standing on `g`'s own proof is real evidence -- `f` must still earn a genuine \
         `bounded(2)`, not fall to `timeout` or `unclaimed`: {}",
        run.json
    );

    let f_statuses: Vec<&str> = f["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        !f_statuses.contains(&"conditional"),
        "`f` stands on `g`'s own clean proof this run, not an assumption -- it must not carry \
         `conditional`: {f}"
    );
    assert!(
        !f_statuses.contains(&"owed-evidence"),
        "and it owes nothing for `g` -- `g` was actually proved, not merely trusted: {f}"
    );

    let root_statuses: Vec<&str> = run.json["root"]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        !root_statuses.contains(&"conditional"),
        "no `conditional` status anywhere in the tree, since nothing in it is: {}",
        run.json
    );

    // A clean verdict is not a standalone one (§5.5): the dependency on
    // `g`'s proof still has to be visible somewhere a reader can see it.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let dep = diagnostics
        .iter()
        .find(|d| d["code"] == "W0517")
        .unwrap_or_else(|| panic!("no W0517 diagnostic recording the dependency: {}", run.json));
    assert_eq!(dep["severity"], "info", "diag: {dep}");
    let title = dep["title"].as_str().unwrap();
    assert!(
        title.contains('g'),
        "must name the callee stood on: {title}"
    );

    // The generated harness must use Kani's own stub-verified mechanism,
    // never inline `g`'s real body and never a hand-built stand-in.
    let generated = std::fs::read_to_string(fixture.path().join("src/ply_generated.rs")).unwrap();
    assert!(
        generated.contains("#[kani::stub_verified(g)]"),
        "generated harness:\n{generated}"
    );
    assert!(
        !generated.contains("#[kani::stub(g,"),
        "must not fall back to the assumed-contract stub mechanism when the callee was actually \
         proved: {generated}"
    );

    assert_eq!(
        run.exit_code,
        Some(0),
        "real, unconditional evidence exits clean: {}",
        run.json
    );
}
