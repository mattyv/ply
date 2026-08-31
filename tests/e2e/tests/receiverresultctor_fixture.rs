//! Defect 1, found pointing Ply at `semver` (docs/reach-measurement-2.md):
//! `A::new` returns `Result<A, Bad>`, and the very same run that builds an
//! `A` *parameter* for `read_it` by calling it used to report `A::doubled`
//! -- a `&self` method on the very same type, in the very same file -- as
//! having no constructor Ply could call. Same constructor, two different
//! answers in one run: the receiver path's own scan recognised only a
//! bare-`Self`-returning constructor, and never learned the `Result<Self,
//! E>` widening the parameter path already had.
//!
//! Also pins all four ways that shape can be spelled (`Self`/the type's
//! own name, crossed with bare/`Result`-wrapped) -- the receiver path must
//! treat every one as the same constructor, not recognise some and miss
//! others.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_result_returning_constructor_builds_a_receiver_in_every_spelling() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverresultctor");

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

    // The measurement's own reproduction: a `Result<A, Bad>` constructor
    // used successfully to build a parameter must also build a receiver,
    // in the same run.
    assert_eq!(
        verdict_of("A::doubled"),
        "fuzzed(64)",
        "`A::new` returns `Result<A, Bad>` -- the same shape `read_it`'s own parameter is built \
         through in this run, so `A::doubled`'s receiver must be built the same way, not refused: \
         {}",
        run.json
    );
    assert_eq!(
        verdict_of("read_it"),
        "fuzzed(64)",
        "envelope: {}",
        run.json
    );

    // All four spellings of the same constructor shape.
    for (node, why) in [
        ("BareSelf::doubled", "`-> Self`"),
        ("ExplicitName::doubled", "`-> ExplicitName`"),
        ("ResultSelf::doubled", "`-> Result<Self, CtorErr>`"),
        (
            "ResultExplicitName::doubled",
            "`-> Result<ResultExplicitName, CtorErr>`",
        ),
    ] {
        assert_eq!(
            verdict_of(node),
            "fuzzed(64)",
            "{node}'s constructor is spelled {why} -- the receiver path must recognise it: {}",
            run.json
        );
    }

    // No node here may carry the false "no associated function ... builds
    // a `<Type>` value" refusal (`V0507`) -- every type has a real,
    // callable constructor.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let false_refusals: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["code"] == "V0507")
        .collect();
    assert!(
        false_refusals.is_empty(),
        "every receiver here has a real, callable constructor -- a `V0507` \"no constructor\" \
         refusal on any of them is false: {:?}",
        false_refusals
    );
}
