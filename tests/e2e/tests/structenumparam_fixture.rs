//! Struct and enum **parameters** (this task, 2026-08-27):
//! `docs/review-self-construction.md`'s rule, applied to an ordinary
//! parameter instead of `&self`. See `tests/fixtures/structenumparam/src/lib.rs`
//! for the fixture's own module doc laying out the three rules this pins.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["root"]["children"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("no fn nodes: {json}"))
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node `{id}`: {json}"))
}

fn diag_for<'a>(json: &'a serde_json::Value, id: &str) -> Vec<&'a serde_json::Value> {
    let qualified = format!("structenumparam::{id}");
    json["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("no diagnostics: {json}"))
        .iter()
        .filter(|d| d["node_id"] == qualified.as_str())
        .collect()
}

/// A function taking an all-public-fields struct (`Point`) earns a real
/// verdict.
#[test]
fn a_function_taking_an_all_public_fields_struct_earns_a_real_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structenumparam");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "manhattan_norm");
    assert_eq!(
        n["verdict"], "fuzzed(64)",
        "`Point`'s fields are all public -- direct construction (rule 2) should build it and \
         earn a real verdict: {}",
        run.json
    );

    // The named assumption this rests on must be disclosed, not left
    // implicit (docs/review-self-construction.md: "keep the rule, say so on
    // every verdict that rested on the assumption").
    let diags = diag_for(&run.json, "manhattan_norm");
    assert!(
        diags.iter().any(|d| d["code"] == "W0522"),
        "a value built by direct field construction must disclose the 'no invariant' \
         assumption it rests on: {}",
        run.json
    );
}

/// A function taking an enum whose variants are all public (`Shape`) earns
/// a real verdict.
#[test]
fn a_function_taking_an_enum_earns_a_real_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structenumparam");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "shape_area_upper_bound");
    assert_eq!(
        n["verdict"], "fuzzed(64)",
        "`Shape`'s variants all carry public fields -- direct construction (rule 2) should \
         build any of them and earn a real verdict: {}",
        run.json
    );
    let diags = diag_for(&run.json, "shape_area_upper_bound");
    assert!(
        diags.iter().any(|d| d["code"] == "W0522"),
        "an enum built by direct variant construction must disclose the same assumption a \
         struct's own direct construction does: {}",
        run.json
    );
}

/// A function taking a struct built via its own constructor (`TicketPool`,
/// private field) earns a real verdict.
#[test]
fn a_function_taking_a_struct_built_via_its_constructor_earns_a_real_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structenumparam");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "doubled_capacity");
    assert_eq!(
        n["verdict"], "fuzzed(64)",
        "`TicketPool`'s one field is private, but `TicketPool::new` is a usable constructor \
         (rule 1) -- it should build a value and earn a real verdict: {}",
        run.json
    );
    // Constructor-built values carry no "no invariant" assumption -- every
    // value really was built by calling the type's own code, so this
    // disclosure must NOT fire here.
    let diags = diag_for(&run.json, "doubled_capacity");
    assert!(
        !diags.iter().any(|d| d["code"] == "W0522"),
        "a constructor-built value needs no invariant-free disclosure -- nothing here was \
         assumed: {}",
        run.json
    );
}

/// **The decisive test**: a genuinely broken function taking a struct built
/// via its constructor is CAUGHT. A passing check would prove nothing.
#[test]
fn a_genuinely_broken_function_taking_a_constructor_built_struct_is_caught() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structenumparam");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "broken_doubled_capacity");
    assert_eq!(
        n["verdict"], "violation",
        "`broken_doubled_capacity`'s promise (`*result == 999999`) is false on every input -- \
         `fuzz(64)` must catch it, not report a clean pass: {}",
        run.json
    );

    let diags = diag_for(&run.json, "broken_doubled_capacity");
    assert!(
        diags
            .iter()
            .any(|d| d["severity"] == "error" || d["code"] == "W0541"),
        "a caught violation must carry a witness diagnostic naming what happened: {}",
        run.json
    );
}

/// **The other decisive test**: a type whose invariant is maintained by its
/// own constructor (`Bucket`, both fields private, `new` always starts full)
/// is never handed an impossible value. If Ply had filled `Bucket`'s
/// private fields in directly, it could build `Bucket { capacity: 1,
/// tokens: 999 }` -- a state the real program can never produce -- and
/// this check would fail on a false alarm. This proves it does not,
/// checking both the reported verdict AND the actual generated harness
/// source, which can only ever call `Bucket::new` (the fields are private,
/// so a field literal would not even compile from this separate crate).
#[test]
fn a_constructor_maintained_invariant_is_never_handed_an_impossible_value() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structenumparam");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "tokens_never_exceed_capacity");
    assert_eq!(
        n["verdict"], "fuzzed(64)",
        "every `Bucket` Ply can build starts with `tokens == capacity` (private fields force \
         the constructor route) -- `tokens <= capacity` must hold on every one of them, cleanly, \
         never a false-alarm violation: {}",
        run.json
    );

    // Read the generated harness source directly (CLAUDE.md: assert the
    // observable outcome, not just the reported verdict) -- it must call
    // `Bucket::new`, and must never construct a `Bucket { .. }` literal at
    // all (which would not even compile here: the fields are private to
    // the fixture crate, not this separate harness crate).
    let harness_dir = fixture.path().join("target/ply/fuzz");
    let harness_src = std::fs::read_dir(&harness_dir)
        .unwrap_or_else(|_| panic!("no harness dir at {}", harness_dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("src/lib.rs"))
        .find(|p| p.is_file())
        .map(|p| std::fs::read_to_string(&p).unwrap())
        .unwrap_or_else(|| panic!("no generated harness src/lib.rs found"));
    assert!(
        harness_src.contains("Bucket::new("),
        "the generated harness must build `Bucket` by calling its own constructor:\n{harness_src}"
    );
    assert!(
        !harness_src.contains("Bucket {") && !harness_src.contains("Bucket{"),
        "the generated harness must never construct a `Bucket` field literal -- both fields are \
         private, so that would not even compile from this separate crate, and Ply must never \
         attempt it:\n{harness_src}"
    );
}

/// A type with no usable constructor and private fields is refused by
/// name.
#[test]
fn a_type_with_no_constructor_and_private_fields_is_refused_by_name() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structenumparam");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "read_secret");
    assert_eq!(
        n["verdict"], "unsupported",
        "`Locked` has no constructor Ply can call and a private field -- `read_secret` must be \
         refused, not silently attempted: {}",
        run.json
    );

    let diags = diag_for(&run.json, "read_secret");
    let d = diags
        .iter()
        .find(|d| d["code"] == "V0509")
        .unwrap_or_else(|| panic!("no V0509 diagnostic naming the refused type: {}", run.json));
    let title = d["title"].as_str().unwrap_or("");
    assert!(
        title.contains("Locked") && title.contains("private"),
        "the refusal must name the type and say why (no constructor, private field): {title}"
    );
}

/// Everything currently refused for other reasons stays refused: a `&mut`
/// parameter is unrelated to struct/enum support and must still be
/// refused.
#[test]
fn a_mut_reference_parameter_stays_refused_for_its_own_reason() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structenumparam");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "bump_mut");
    assert_eq!(
        n["verdict"], "unsupported",
        "a `&mut` parameter is refused for a reason this task did not touch, and must stay \
         refused: {}",
        run.json
    );
    let diags = diag_for(&run.json, "bump_mut");
    assert!(
        diags.iter().any(|d| d["code"] == "V0505"),
        "the existing `&mut` refusal must still fire: {}",
        run.json
    );
}

/// The fixture crate itself must still build and run its own (empty) test
/// suite after `cargo ply verify` has generated and run every harness --
/// Ply must never leave a user's crate unbuildable.
#[test]
fn the_fixture_crate_itself_still_builds_after_verify() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structenumparam");
    let _run = run_verify(&cargo_ply, fixture.path(), 90);

    let status = std::process::Command::new("cargo")
        .args(["test", "--lib"])
        .current_dir(fixture.path())
        .status()
        .expect("spawning `cargo test --lib` in the fixture crate");
    assert!(
        status.success(),
        "the fixture crate must still compile and run after `cargo ply verify`"
    );
}
