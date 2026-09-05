//! Five ordinary shapes that used to make a generated harness fail to
//! compile, reported as a raw-compiler-output tool error instead of a named
//! refusal (docs/review-structs-enums.md's "Also fix" list, 2026-08-28):
//! a non-public type, a private constructor beside public fields, a struct
//! with 13 public fields, `#[non_exhaustive]` on one variant, and a type
//! behind a private module. Each must now be refused by name before
//! generation -- except the private-constructor one, which has a working
//! fallback (direct field construction) and must actually be checked -- and
//! `normal_fn`'s own real bug must still be found, proving the rest of the
//! crate stays checkable.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn each_shape_is_refused_by_name_and_the_rest_of_the_crate_stays_checkable() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("fiveshapes");
    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("no component children: {}", run.json));
    let verdict_of = |id: &str| -> String {
        children
            .iter()
            .find(|c| c["id"] == id)
            .unwrap_or_else(|| panic!("no node for {id}: {}", run.json))["verdict"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // None of the four genuinely-unbuildable shapes may ever read as a tool
    // error: each must be refused honestly, before generation.
    for id in ["uses_hidden", "uses_status", "uses_quota"] {
        let v = verdict_of(id);
        assert!(
            v.starts_with("unsupported"),
            "`{id}` must be refused by name before generation, not reach a broken harness: got \
             `{v}`: {}",
            run.json
        );
    }

    // The private-constructor shape has a working fallback (direct field
    // construction, since every field is already public) -- refusing it
    // would throw that away.
    assert_eq!(
        verdict_of("uses_privctor"),
        "fuzzed(64)",
        "a private constructor beside public fields must fall back to direct field \
         construction and actually be checked, not be refused: {}",
        run.json
    );

    // The one real bug in this crate, sharing whatever harness the five
    // broken shapes above share, must still be found -- proving none of
    // them poisoned it.
    assert_eq!(
        verdict_of("normal_fn"),
        "violation",
        "`normal_fn`'s real bug (false at n=0) must still be found even though five other \
         functions in this crate hit shapes that used to break the shared harness: {}",
        run.json
    );

    // No X0901 tool-error diagnostic (raw compiler output) belongs anywhere
    // in this run.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().all(|d| d["code"] != "X0901"),
        "no shape here should ever reach codegen and fail to compile: {}",
        run.json
    );

    // Each refusal must name what actually blocked it, not a generic
    // message -- the newbie bar this task's own diagnostics are held to.
    let title_for = |id: &str| -> String {
        diagnostics
            .iter()
            .find(|d| d["node_id"] == format!("fiveshapes::{id}") && d["code"] == "V0509")
            .unwrap_or_else(|| panic!("no V0509 refusal for {id}: {}", run.json))["title"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(
        title_for("uses_hidden").contains("not `pub`"),
        "{}",
        title_for("uses_hidden")
    );
    // `uses_big13` used to be here, asserting the refusal by name. Ply
    // refused a struct with more than twelve public fields because its
    // generated recipe was one flat tuple and the sampling library's trait
    // for those stops at twelve -- a fact about Ply's folding, not about
    // the struct. The tuple nests now, and the assertion moved to
    // `a_struct_wider_than_a_flat_tuple_allows_is_checked_not_refused`
    // below, which proves the thirteenth field is drawn rather than
    // silently defaulted.
    assert!(
        title_for("uses_status").contains("Weird")
            && title_for("uses_status").contains("non_exhaustive"),
        "{}",
        title_for("uses_status")
    );
    assert!(
        title_for("uses_quota").contains("quota"),
        "{}",
        title_for("uses_quota")
    );

    // The private constructor must be named too, in its own (non-refusal)
    // disclosure -- found and explained, not silently invisible.
    let privctor_disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "fiveshapes::uses_privctor" && d["code"] == "W0522")
        .unwrap_or_else(|| panic!("no W0522 disclosure for uses_privctor: {}", run.json));
    let privctor_title = privctor_disclosure["title"].as_str().unwrap();
    assert!(
        privctor_title.contains("WithPrivateCtor::new") && privctor_title.contains("private"),
        "{privctor_title}"
    );
}

/// The shape that used to be refused, and why the refusal was wrong.
///
/// Ply declined any struct with more than twelve public fields, saying so
/// as if it were a fact about the struct. It was a fact about Ply: a
/// struct's generated recipe was one flat tuple, and the sampling
/// library's trait for tuples stops at twelve. Nesting the tuple in chunks
/// removes the limit, which was measured against the pinned library
/// version before this changed (2026-09-04).
///
/// Asserting that it now *compiles* would prove almost nothing -- a
/// codegen that quietly filled every field past the twelfth with a default
/// would also compile, and report a confident green over a space it never
/// explored. So the fixture's promise is false only when its **thirteenth**
/// field is large, and this asserts the violation is found. That can only
/// happen if the thirteenth leaf is genuinely being drawn.
#[test]
fn a_struct_wider_than_a_flat_tuple_allows_is_checked_not_refused() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("fiveshapes");
    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("no component children: {}", run.json));
    let node = children
        .iter()
        .find(|c| c["id"] == "uses_big13")
        .unwrap_or_else(|| panic!("no node for uses_big13: {}", run.json));
    assert_eq!(
        node["verdict"].as_str().unwrap(),
        "violation",
        "a promise false only on the thirteenth field must be caught, or that \
         leaf is not being drawn: {}",
        run.json
    );
}
