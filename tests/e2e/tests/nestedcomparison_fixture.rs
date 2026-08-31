//! Defect 2, found pointing Ply at `semver` (docs/reach-measurement-2.md):
//! `same`'s postcondition states a boolean result as an equality of two
//! other equalities (`*result == (a == b)`), and the harness Ply generated
//! for it did not compile -- `error[E0308]: mismatched types`, because the
//! postcondition widening cast the nested comparison's tokens to `i128`
//! with no parens of its own, and `as` binds tighter than `==`.
//!
//! Also covers a comparison nested under `&&`, one nested under `||`, a
//! comparison of two expressions rather than two bare names, and a mixed
//! case with arithmetic on one side -- all five needed the fix. Two more
//! (`bigger`, `either`) put `&&`/`||` at the postcondition's own outermost
//! position, with no wrapping equality -- these already worked before the
//! fix (`widen`'s own recursion already handles that shape), and are here
//! to prove the fix did not need to touch that path.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_comparison_nested_inside_another_comparison_no_longer_fails_to_compile() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nestedcomparison");

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
        ("same", "the reported repro, verbatim: a comparison of two bare names nested inside another comparison"),
        ("both_eq", "a comparison nested under `&&`, wrapped by an outer `==`"),
        ("either_eq", "a comparison nested under `||`, wrapped by an outer `==`"),
        ("halves_eq", "a comparison of two expressions, not two bare names"),
        ("bumped_eq", "arithmetic on one side of the nested comparison"),
        ("bigger", "`&&` at the postcondition's own outermost position -- already worked before this fix"),
        ("either", "`||` at the postcondition's own outermost position -- already worked before this fix"),
    ] {
        assert_eq!(
            verdict_of(node),
            "fuzzed(64)",
            "{node} ({why}) -- the generated harness must actually compile and run, never \
             mismatch types from a mis-parenthesised cast: {}",
            run.json
        );
    }

    // No node here may end in the tool error a mis-parenthesised widened
    // comparison produces.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().all(|d| d["code"] != "X0901"),
        "no run may end in a compile-failure tool error here -- every one of these harnesses must \
         actually compile: {}",
        run.json
    );
}
