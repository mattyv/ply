//! Defect 2, found pointing Ply at `semver` (docs/reach-measurement-2.md):
//! `Pair::value_of` takes another `&Pair` -- its own receiver's type -- and
//! the harness Ply generated for it imported `Pair` twice, once for the
//! receiver and once for the parameter, which is `error[E0252]: the name
//! `Pair` is defined multiple times`. `checks: [fuzz(64)]` on a method like
//! that used to run zero cases and report a tool error instead of a real
//! verdict.
//!
//! Also covers two parameters sharing a type with no receiver involved, and
//! a parameter's type also being the return type -- both can produce the
//! same duplicate `use`. (Neither postcondition in the fixture reads `self`
//! -- see its own module doc for the separate, pre-existing gap that
//! surfaces once this defect no longer masks it.)

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_parameter_sharing_its_receivers_type_does_not_double_import() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("sharedtypeparam");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let verdict_of = |node_id: &str| -> String {
        fn find<'a>(n: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
            if n["id"] == id {
                return Some(n);
            }
            n["children"]
                .as_array()?
                .iter()
                .find_map(|child| find(child, id))
        }
        find(&run.json["root"], node_id)
            .unwrap_or_else(|| panic!("no node `{node_id}` in envelope: {}", run.json))["verdict"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };

    for (node, why) in [
        (
            "Pair::value_of",
            "a `&self` method taking another `&Pair` -- the receiver's own type",
        ),
        (
            "Pair::copy_of",
            "the receiver's type, a parameter's type, and the return type (`Self`) all naming \
             the same struct",
        ),
        (
            "both_same",
            "two parameters of the same struct type, no receiver involved",
        ),
    ] {
        assert_eq!(
            verdict_of(node),
            "fuzzed(64)",
            "{node} shares one struct type across {why} -- the generated harness must import that \
             type once, not fail to compile from importing it twice: {}",
            run.json
        );
    }

    // No node here may end in the tool error a duplicate `use` produces.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().all(|d| d["code"] != "X0901"),
        "no run may end in a compile-failure tool error here -- every one of these harnesses must \
         actually compile: {}",
        run.json
    );
}
