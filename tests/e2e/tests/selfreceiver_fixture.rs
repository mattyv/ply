//! Defect 1, found pointing Ply at `semver` (docs/reach-measurement-2.md):
//! `Pair::bumped`'s postcondition reads `self.a`, and the harness Ply
//! generated for it did not compile -- `error[E0424]: expected value,
//! found module `self``, because the postcondition is spliced into the
//! generated test as a free-standing expression outside any `impl` block,
//! where the literal keyword `self` means nothing. This is the most
//! natural thing a method's own promise can say, so `checks: [fuzz(64)]`
//! on almost any real method used to end in a tool error instead of a real
//! verdict.
//!
//! Also covers `self` read alongside a parameter, and `self` read on a
//! receiver built through a fallible (`Result<Self, E>`) constructor --
//! the shape the *other* defect fixed the same day (`receiverresultctor`)
//! made buildable, now interacting with this fix.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_postcondition_reading_self_no_longer_fails_to_compile() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("selfreceiver");

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
            "Pair::bumped",
            "the reported repro, verbatim: `self` and the result together",
        ),
        (
            "Pair::at_least",
            "`self` and a parameter read together in one postcondition",
        ),
        (
            "Meter::doubled",
            "a receiver built through a fallible (`Result<Self, E>`) constructor, whose own \
             postcondition also reads `self`",
        ),
    ] {
        assert_eq!(
            verdict_of(node),
            "fuzzed(64)",
            "{node}'s postcondition reads `self` ({why}) -- the generated harness must actually \
             compile and run, not fail with `self` meaning nothing outside an `impl` block: {}",
            run.json
        );
    }

    // No node here may end in the tool error a bad `self` splice produces.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().all(|d| d["code"] != "X0901"),
        "no run may end in a compile-failure tool error here -- every one of these harnesses must \
         actually compile: {}",
        run.json
    );
}
