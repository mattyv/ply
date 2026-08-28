//! Two more ordinary ways to write Rust that Ply refused, each producing
//! the same false sentence: "it has no constructor Ply can call", about a
//! type whose public `new` sits a few lines away.
//!
//! An `impl` block inside an inline `mod` in the same file was never looked
//! at -- the scan walked only the file's top-level items. And a parameter
//! spelled `crate::Beta` rather than `Beta` was carried around as the
//! rendering of a token stream, `crate :: Beta`, which the by-bare-name
//! lookup could never match.
//!
//! Neither is exotic. Both were found by writing the code an ordinary Rust
//! programmer writes and watching Ply be wrong about it, which is also why
//! the assertions below are per-claim: a worst-of root verdict would go
//! green as soon as either one worked.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_inline_module_impl_and_a_qualified_parameter_are_both_ordinary_and_both_checked() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("ordinaryspellings");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    let verdicts = leaf_verdicts(&run.json);
    for fn_name in ["alpha_inline_mod", "beta_qualified"] {
        assert_eq!(
            verdicts.get(fn_name).map(String::as_str),
            Some("fuzzed(64)"),
            "`{fn_name}`'s type has a public constructor, so the claim must be checked: {}",
            run.json
        );
    }
    assert_eq!(
        run.json["diagnostics"].as_array().unwrap().len(),
        0,
        "nothing here is refused any more: {}",
        run.json
    );

    // Green is only worth something if Ply really built the values. Weaken
    // what both constructors guarantee and both claims have to fail.
    let src_path = fixture.path().join("src/lib.rs");
    let src = std::fs::read_to_string(&src_path).unwrap();
    let weakened = src.replace("n: n.max(1)", "n");
    assert_ne!(
        weakened, src,
        "the edit this test rests on matched nothing in src/lib.rs"
    );
    std::fs::write(&src_path, weakened).unwrap();

    let run2 = run_verify(&cargo_ply, fixture.path(), 120);
    let after = leaf_verdicts(&run2.json);
    for fn_name in ["alpha_inline_mod", "beta_qualified"] {
        assert_eq!(
            after.get(fn_name).map(String::as_str),
            Some("violation"),
            "if weakening the constructor changes nothing for `{fn_name}`, Ply never called \
             it: {}",
            run2.json
        );
    }
}

/// Every leaf claim's verdict, keyed by the `id` the envelope gives it.
fn leaf_verdicts(envelope: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    fn walk(node: &serde_json::Value, out: &mut std::collections::BTreeMap<String, String>) {
        match node["children"].as_array() {
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
