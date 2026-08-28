//! A constructor in a **qualified** `impl` block, for a type used as an
//! ordinary parameter.
//!
//! The receiver path learned in 2026-08-27 that `impl super::T`,
//! `impl crate::T` and `impl Alias` name the same type as `impl T` -- found
//! after a mutator written that way was never called and the verdict
//! claimed coverage it did not have. The parameter path kept the older
//! rule, which counted an `impl` block only when its self type was a single
//! bare segment.
//!
//! So a type declared in `lib.rs` with its `impl` block in a submodule --
//! which must write `impl super::T`, there being no other spelling
//! available in that file -- had no constructor as far as Ply was
//! concerned, and Ply said so: "it has no constructor Ply can call", about
//! a type with a public `new`. Not a missing feature but a false sentence,
//! which is the worse of the two.
//!
//! Probing the same family turned up a second spelling with the same
//! effect, on both paths: a constructor that returns the type by name
//! (`-> super::Quota`) rather than `Self`. Only `Self` was recognised, so
//! that constructor was invisible too -- for an ordinary parameter and for
//! a receiver alike, the receiver message reading "none was found".
//!
//! Both halves are pinned here for both paths: the claims are checked at
//! all, and the checks are real rather than vacuous -- weakening what the
//! constructor guarantees has to turn both verdicts red, or Ply is not
//! actually calling it.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_constructor_in_a_qualified_impl_block_is_found_and_actually_called() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("qualifiedctor");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the constructor is reachable, so both claims must be checked: {}",
        run.json
    );
    // Both paths, not just the one that first exposed the bug: an ordinary
    // parameter of the type, and a method that needs it as a receiver.
    let verdicts = leaf_verdicts(&run.json);
    assert_eq!(
        verdicts.get("refill_per_second").map(String::as_str),
        Some("fuzzed(64)"),
        "the parameter path: {}",
        run.json
    );
    assert_eq!(
        verdicts.get("Quota::burst_ceiling").map(String::as_str),
        Some("fuzzed(64)"),
        "the receiver path, which had the same blind spot: {}",
        run.json
    );

    // The half that matters. A green run here would be worthless if Ply
    // were not really constructing the value: break what the constructor
    // guarantees and the promise resting on it has to fail.
    let ops = fixture.path().join("src/ops.rs");
    let ops_src = std::fs::read_to_string(&ops).unwrap();
    let weakened = ops_src.replace("per_second: per_second.max(1),", "per_second,");
    assert_ne!(
        weakened, ops_src,
        "the edit this test rests on did not match anything in src/ops.rs"
    );
    std::fs::write(&ops, weakened).unwrap();

    let run2 = run_verify(&cargo_ply, fixture.path(), 120);
    let after = leaf_verdicts(&run2.json);
    for fn_name in ["refill_per_second", "Quota::burst_ceiling"] {
        assert_eq!(
            after.get(fn_name).map(String::as_str),
            Some("violation"),
            "if weakening the constructor changes nothing for `{fn_name}`, Ply never called \
             it there: {}",
            run2.json
        );
    }
}

/// Every leaf claim's verdict, keyed by the `id` the envelope gives it -- so each path can be asserted on separately rather than through
/// one worst-of root that either could satisfy alone.
fn leaf_verdicts(envelope: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    fn walk(node: &serde_json::Value, out: &mut std::collections::BTreeMap<String, String>) {
        let children = node["children"].as_array();
        match children {
            Some(kids) if !kids.is_empty() => {
                for k in kids {
                    walk(k, out);
                }
            }
            _ => {
                if let (Some(name), Some(verdict)) = (node["id"].as_str(), node["verdict"].as_str())
                {
                    out.insert(name.to_string(), verdict.to_string());
                }
            }
        }
    }
    walk(&envelope["root"], &mut out);
    out
}
