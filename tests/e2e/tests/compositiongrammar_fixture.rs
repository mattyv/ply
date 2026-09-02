//! The invariant test CLAUDE.md asks for (see `tools/render/tests/render.rs`'s
//! `every_painted_element_resolves_a_style_rule`): walk every shape the
//! composition grammar (TODO.md, "make the sampling engine's decision
//! recursive", 2026-09-02) admits, to a small bound, and require every one
//! to yield something that actually compiles and runs -- never just a
//! plausible-looking generated file. `compositiongrammar`'s own 29
//! functions are that walk (see its own `src/lib.rs` doc for exactly which
//! shapes); this test walks the *real output* of a real run over all of
//! them and fails on the first one that did not earn a real verdict, named,
//! so a construct added to the grammar later that quietly breaks one
//! combination cannot pass silently.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn every_shape_the_grammar_admits_compiles_and_earns_real_evidence() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("compositiongrammar");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let mut fn_nodes: Vec<&serde_json::Value> = Vec::new();
    fn collect<'a>(node: &'a serde_json::Value, out: &mut Vec<&'a serde_json::Value>) {
        if node["kind"] == "fn" {
            out.push(node);
        }
        if let Some(children) = node["children"].as_array() {
            for c in children {
                collect(c, out);
            }
        }
    }
    collect(&run.json["root"], &mut fn_nodes);

    assert!(
        !fn_nodes.is_empty(),
        "the walk found no functions at all -- the fixture itself failed to load: {}",
        run.json
    );

    // Fail on the *first* unexplained shape, naming it -- never a summary
    // count that leaves the reader to go hunting.
    for node in &fn_nodes {
        let verdict = node["verdict"].as_str().unwrap_or("");
        assert!(
            verdict.starts_with("fuzzed("),
            "`{}` did not compile and run for real (verdict `{verdict}`) -- every shape this \
             grammar admits must yield something that actually builds, not merely parses. Full \
             run: {}",
            node["id"],
            run.json
        );
    }

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(8)",
        "every one of the 29 shapes carries the same trivial, always-true contract, so the \
         whole run's own verdict must be a clean pass too: {}",
        run.json
    );
}
