//! D1 (adversarial review, 2026-08-26): branch one's min-composition is
//! sound only when the callee's own proof covers its *entire* argument
//! space. A `Vec<u8>` parameter breaks that: the callee's `bounded(k)`
//! proof only ever builds vectors up to length `k`, so a caller passing a
//! longer one gets the contract assumed on an input the proof never
//! covered. Before this was excluded, this exact fixture composed to a
//! false clean `bounded(2)`, exit 0, while the real function violated its
//! own postcondition on every input -- the worst failure mode this project
//! exists to refuse, and the parent commit's own reported an honest
//! `timeout` instead.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_callee_with_a_vec_parameter_never_qualifies_for_the_first_branch() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverifiedveclen");

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
        "g's own proof is real -- it just does not cover every vector f could pass: {}",
        run.json
    );

    // The point of the whole test: f must never be reported clean here.
    assert_ne!(
        f["verdict"], "violation",
        "a Vec-parameter callee falling back to branch two must not itself cause a false \
         violation either -- the assumed contract is what f's own proof rests on: {}",
        run.json
    );
    assert_eq!(
        f["verdict"], "bounded(2)",
        "f's own proof is still real evidence -- standing on g's *assumed* contract, not its \
         body: {}",
        run.json
    );
    let f_statuses: Vec<&str> = f["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        f_statuses.contains(&"conditional"),
        "g's Vec<u8> parameter means its bounded(2) proof does not cover every argument f could \
         pass, so f must never be reported clean standing on it -- a Vec-parameter callee is \
         never eligible for branch one: {f}"
    );
    assert!(
        f_statuses.contains(&"owed-evidence"),
        "and the assumption is owed evidence exactly like any other branch-two verdict: {f}"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "W0517"),
        "no clean-dependency diagnostic may appear when the callee was excluded from branch \
         one: {}",
        run.json
    );
    assert!(
        diagnostics.iter().any(|d| d["code"] == "W0511"),
        "the ordinary conditional-verdict diagnostic must still be present: {}",
        run.json
    );

    assert_eq!(
        run.exit_code,
        Some(0),
        "`conditional` is real evidence and exits clean: {}",
        run.json
    );
}
