//! Method resolution (The-Ply-Spec.md §5.2, §5.1a rule 3): `Type::method`
//! now resolves to the right item inside an `impl` block.
//!
//! Before this, `crates/ply-core/src/callgraph.rs` indexed only free
//! functions -- no `syn::Item::Impl` arm existed at all -- so every claim in
//! this fixture reported `E0301` ("could not find the function"), a false
//! statement about every one of them: the function is right there.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["root"]["children"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("no fn nodes: {json}"))
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node `{id}`: {json}"))
}

/// Diagnostics carry the full component-qualified `node_id`
/// (`implmethod::Bucket::capacity`), unlike a tree node's own `id`
/// (`Bucket::capacity`, see `leaf_node`'s doc) -- so this qualifies `id`
/// with the fixture's one component before matching.
fn diag_for<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    let qualified = format!("implmethod::{id}");
    json["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("no diagnostics: {json}"))
        .iter()
        .find(|d| d["node_id"] == qualified.as_str())
        .unwrap_or_else(|| panic!("no diagnostic for `{qualified}`: {json}"))
}

#[test]
fn a_receiverless_associated_function_is_fully_checked_and_earns_a_real_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Bucket::new");
    assert_eq!(
        n["verdict"], "bounded(2)",
        "a receiverless associated function has no receiver to construct -- nothing but \
         resolution ever blocked it, and it must earn the same real verdict a free function \
         would: {}",
        run.json
    );
}

#[test]
fn a_method_with_a_receiver_resolves_and_is_refused_by_name_not_silently_missing() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Bucket::capacity");
    assert_eq!(
        n["verdict"], "unsupported",
        "a method with a receiver is real and resolved; it must not silently report as if \
         nothing were there: {}",
        run.json
    );
    let d = diag_for(&run.json, "Bucket::capacity");
    assert_ne!(
        d["code"], "E0301",
        "E0301 says \"could not find the function\", which is false here -- Ply found \
         `Bucket::capacity`: {d}"
    );
    let title = d["title"].as_str().unwrap();
    assert!(
        !title.contains("could not find"),
        "the diagnostic must not claim Ply could not find a function it actually resolved: {d}"
    );
    assert!(
        title.contains("Bucket::capacity") && title.contains("receiver"),
        "the diagnostic must name the function and say plainly that a receiver is the reason \
         it is refused: {d}"
    );
}

#[test]
fn a_free_function_and_a_method_sharing_a_name_never_resolve_to_each_other() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // The free function `capacity` is receiverless and contracted: it must
    // be checked as itself, never confused with the method `Bucket::capacity`
    // (checked with no explicit checks, refused for its receiver above).
    let free_fn = node(&run.json, "capacity");
    assert_eq!(
        free_fn["verdict"], "bounded(2)",
        "the free function `capacity` must resolve and check on its own terms, not inherit \
         `Bucket::capacity`'s receiver refusal: {}",
        run.json
    );
    let method = node(&run.json, "Bucket::capacity");
    assert_eq!(
        method["verdict"], "unsupported",
        "the method `Bucket::capacity` must stay refused for its own reason, not borrow the \
         free function's clean verdict: {}",
        run.json
    );
}

#[test]
fn two_impl_blocks_for_one_type_with_no_name_collision_both_resolve() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // `Meter::zero` (first impl block) and `Meter::centimeters_per_meter`
    // (second impl block) must both resolve and check independently --
    // multiple `impl` blocks for one type is ordinary Rust, not an
    // ambiguity, as long as they do not define the same name twice.
    let zero = node(&run.json, "Meter::zero");
    assert_eq!(
        zero["verdict"], "fuzzed(32)",
        "the first impl block's own method must resolve and check independently: {}",
        run.json
    );
    let cpm = node(&run.json, "Meter::centimeters_per_meter");
    assert_eq!(
        cpm["verdict"], "bounded(2)",
        "the second impl block's own method must resolve and check independently: {}",
        run.json
    );
}

#[test]
fn a_trait_method_and_a_trait_impl_method_are_refused_by_name_not_missing() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    for id in ["Widget::size", "Gadget::size"] {
        let n = node(&run.json, id);
        assert_eq!(n["verdict"], "unsupported", "{id}: {}", run.json);
        let d = diag_for(&run.json, id);
        assert_ne!(d["code"], "E0301", "{id}: {d}");
        let title = d["title"].as_str().unwrap();
        assert!(
            title.contains("trait"),
            "`{id}`'s refusal must name the reason (a trait method): {d}"
        );
    }
}

#[test]
fn a_generic_impl_block_is_refused_by_name_not_missing() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Pair::describe");
    assert_eq!(n["verdict"], "unsupported", "{}", run.json);
    let d = diag_for(&run.json, "Pair::describe");
    assert_ne!(d["code"], "E0301", "{d}");
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("generic"),
        "`Pair::describe`'s refusal must name the reason (a generic `impl` block): {d}"
    );
}

/// Adversarial review, 2026-08-27: the fuzz-tier harness crate imported a
/// method the same way it imports a free function -- `use
/// crate::Bucket::clamped;` -- which does not compile (a method is not an
/// importable item). Every method claim checked on the fuzz tier came back
/// `tool_error`/`X0901` ("failed to compile") instead of a real verdict.
#[test]
fn a_receiverless_method_checked_on_the_fuzz_tier_earns_a_real_verdict_not_a_broken_harness() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Bucket::clamped");
    assert_eq!(
        n["verdict"], "fuzzed(32)",
        "a receiverless method checked with `fuzz` must compile and earn a real verdict, not \
         `tool_error`: {}",
        run.json
    );
    assert!(
        !run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["node_id"] == "implmethod::Bucket::clamped"),
        "a clean fuzzed verdict must carry no diagnostic at all: {}",
        run.json
    );
}

/// Adversarial review, 2026-08-27: a receiverless method whose *return*
/// type Ply's parser does not model must be refused honestly as
/// unsupported, the same way an unrecognised *parameter* type already is
/// -- never left to reach codegen and fail some other way.
#[test]
fn a_receiverless_method_with_an_unsupported_return_type_is_refused_not_broken() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Bucket::make_elsewhere");
    assert_eq!(
        n["verdict"], "unsupported",
        "an unrecognised return type must be refused, not attempted: {}",
        run.json
    );
    let d = diag_for(&run.json, "Bucket::make_elsewhere");
    assert_eq!(
        d["code"], "V0505",
        "the existing unsupported-shape diagnostic, not a new one: {d}"
    );
    assert!(
        !d["title"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("tool_error")
            && d["code"] != "X0901",
        "must be reported before codegen runs, never as a broken-harness tool error: {d}"
    );
}

/// Adversarial review, 2026-08-27, found independently of the method-import
/// bug above: a zero-parameter fn checked with `fuzz` built a strategy
/// expression of a bare `()` -- a value, not a `proptest::strategy::Strategy`
/// -- so it failed to compile regardless of whether it was a method or a
/// free function. `FakeClock::new()` in the rate-limiter fixture is this
/// exact shape, which is why this is pinned on its own rather than folded
/// into the import-bug test above (§9: a defect found by review enters the
/// suite as a fixture of its own shape).
#[test]
fn a_zero_parameter_fn_checked_on_the_fuzz_tier_earns_a_real_verdict_not_a_broken_harness() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmethod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Meter::zero");
    assert_eq!(
        n["verdict"], "fuzzed(32)",
        "a zero-parameter fn checked with `fuzz` must compile and earn a real verdict, not \
         `tool_error`: {}",
        run.json
    );
}
