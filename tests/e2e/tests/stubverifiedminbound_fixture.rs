//! D5's first branch, bound composition (The-Ply-Spec.md §5.5): this is the
//! anti-overclaim test and it matters most. `f` declares a deeper bound
//! than the callee it stands on ever proved -- `f`'s proof only holds
//! *given* `g` meets its contract, and that was only established to `g`'s
//! own depth. Reporting `f` at its own declared bound here would be
//! exactly the "evidence stronger than what it rests on" overclaim this
//! project exists to refuse.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_caller_standing_on_a_shallower_proof_reports_the_shallower_bound() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverifiedminbound");

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

    assert_eq!(
        g["verdict"], "bounded(2)",
        "`g` itself is only ever claimed and proved to bounded(2): {}",
        run.json
    );
    assert_eq!(
        f["verdict"], "bounded(2)",
        "`f` declares `bounded(5)`, but it stands on `g`'s bounded(2) proof -- the honest \
         composed verdict is the weaker of the two. Reporting `bounded(5)` here would claim a \
         depth nothing actually checked: {}",
        run.json
    );
    assert_ne!(
        f["verdict"], "bounded(5)",
        "`f`'s own declared bound must never be reported when the callee it rests on only \
         proved a shallower one -- this is the overclaim the whole project exists to prevent: {}",
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
        "the composed bound is still real evidence, standing on a proof, not an assumption: {f}"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let dep = diagnostics
        .iter()
        .find(|d| d["code"] == "W0517")
        .unwrap_or_else(|| panic!("no W0517 diagnostic recording the dependency: {}", run.json));
    let title = dep["title"].as_str().unwrap();
    assert!(
        title.contains("bounded(2)"),
        "the diagnostic must name the depth the dependency actually earned: {title}"
    );

    assert_eq!(run.exit_code, Some(0), "envelope: {}", run.json);
}
