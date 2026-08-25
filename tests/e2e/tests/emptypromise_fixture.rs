//! A promise that says nothing must not buy a confident result.
//!
//! §1's principle, learned again: an absence of real assumption is not a
//! pass. Two shapes, and they fail differently.
//!
//! **Unsatisfiable.** Nothing satisfies `|result| *result > 10_000 && *result
//! < 5`. Ply hands that clause to the engine as an assumption, so the
//! caller's proof holds vacuously and everything downstream of it is
//! provable. `vacuous_fee`'s postcondition is plainly false, and before
//! 2026-08-25 the run reported it as `bounded(2)` -- exit 0, no error, the
//! impossible promise listed beside the verdict as though it were carrying
//! weight. That is the most expensive lie this tool can tell.
//!
//! **Trivially true.** `|result| *result >= 0` is true of every `u32`. It
//! constrains nothing, so the callee is in effect replaced by an arbitrary
//! value -- and the run reported the clause as an assumption owed evidence,
//! which sends a reader off to discharge a debt that does not exist.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["root"]["children"][0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node `{id}` in {json}"))
}

#[test]
fn an_unsatisfiable_promise_never_yields_a_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("emptypromise");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let vacuous = node(&run.json, "vacuous_fee");
    assert_eq!(
        vacuous["verdict"], "unclaimed",
        "`vacuous_fee`'s postcondition is false, and it was reported green only because the \
         promise it rested on cannot be true of anything: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "E0502")
        .unwrap_or_else(|| panic!("no E0502 in {}", run.json));
    assert_eq!(d["severity"], "error", "{d}");
    let title = d["title"].as_str().unwrap();
    assert!(title.contains("legacy_rate"), "{title}");
    assert!(
        title.contains("*result > 10_000 && *result < 5"),
        "the clause a user has to fix must be quoted back to them: {title}"
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "a promise nothing can satisfy must fail the run: {}",
        run.json
    );
}

#[test]
fn a_trivially_true_promise_is_reported_as_saying_nothing() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("emptypromise");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "E0503")
        .unwrap_or_else(|| panic!("no E0503 in {}", run.json));
    assert_eq!(d["severity"], "error", "{d}");
    let title = d["title"].as_str().unwrap();
    assert!(title.contains("legacy_cap"), "{title}");
    assert!(
        title.contains("*result >= 0"),
        "the clause must be quoted back: {title}"
    );
    assert!(
        title.contains("u32"),
        "naming the type is what makes `>= 0` obviously empty rather than merely suspicious: \
         {title}"
    );

    // The caller's own result is real evidence and stays: replacing the
    // callee with an unconstrained value is a *weaker* assumption than the
    // promise, not a stronger one. What was wrong was the report calling it
    // an assumption.
    let havoc = node(&run.json, "havoc_fee");
    assert_eq!(havoc["verdict"], "bounded(2)", "{}", run.json);

    let w0511 = diagnostics
        .iter()
        .find(|d| d["code"] == "W0511" && d["node_id"] == "emptypromise::havoc_fee")
        .unwrap_or_else(|| panic!("no W0511 for havoc_fee in {}", run.json));
    let text = w0511["title"].as_str().unwrap();
    assert!(
        text.contains("constrained nothing"),
        "the `conditional` sentence must not present an empty clause as an assumption that is \
         owed evidence: {text}"
    );
}
