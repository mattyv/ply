//! D5's second branch, proven unchanged (The-Ply-Spec.md §5.5): `g` carries
//! its own inline contract but is claimed with `fuzz`, never `bounded`, so
//! it can never earn the `bounded(k)` verdict D5's first branch requires.
//! `f`'s call to it must still come back `conditional`, exactly as before
//! this feature existed -- this is the regression guard that D5's first
//! branch did not quietly widen to cover callees it has no business
//! covering.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_callee_only_fuzz_checked_never_qualifies_for_the_first_branch() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverifiedfuzzedcallee");

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

    assert!(
        g["verdict"].as_str().unwrap_or("").starts_with("fuzzed("),
        "`g` is only claimed with `fuzz`, so it must earn `fuzzed(n)`, never `bounded`: {}",
        run.json
    );
    assert_eq!(
        f["verdict"], "bounded(2)",
        "`f`'s own proof is still real -- it is the assumption about `g` that is conditional: {}",
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
        "a callee that was never proved `bounded` this run can never qualify for D5's first \
         branch, however it was checked: {f}"
    );
    assert!(f_statuses.contains(&"owed-evidence"), "{f}");

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "W0517"),
        "no clean-dependency diagnostic when the callee was never proved bounded: {}",
        run.json
    );
    assert!(
        diagnostics.iter().any(|d| d["code"] == "W0511"),
        "{}",
        run.json
    );

    assert_eq!(run.exit_code, Some(0), "envelope: {}", run.json);
}
