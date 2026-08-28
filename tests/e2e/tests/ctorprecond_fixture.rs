//! Adversarial review, docs/review-caveats.md N2: Ply built a receiver by
//! calling the type's own constructor with an argument the constructor's
//! own declared precondition forbids, the constructor's own assertion
//! fired, and the crash was reported as the *checked method* breaking its
//! own contract. `Gauge::value`'s promise (`*result >= 0` on a `u32`)
//! cannot be false -- a violation here can only mean Ply itself called
//! `Gauge::new` outside its own precondition.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_unfalsifiable_promise_is_clean_when_the_constructors_precondition_is_honoured() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("ctorprecond");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        verdict != "violation",
        "`*result >= 0` on a `u32` cannot be false -- reporting a violation here means Ply \
         called `Gauge::new` with an argument its own `#[ply::requires(n > 0)]` forbids: {}",
        run.json
    );
    assert!(
        verdict.starts_with("fuzzed"),
        "an unfalsifiable promise, checked honestly, is a real (if uninteresting) pass: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "ctorprecond::Gauge::value" || d["severity"] != "error"),
        "no error-severity diagnostic belongs on correct code: {}",
        run.json
    );
}
