//! Regression e2e for the 2026-08-24 M4 review's D3: §5.4c promises "a
//! warning when the rejection rate is high", and the shipped guard could
//! never fire on its ratio path -- it counted every rejected draw in *both*
//! sides of the fraction, which reduces to "rejected > total", i.e.
//! "accepted < 0". A `requires` that throws away ~2 of every 3 generated
//! inputs must warn, and must still report the evidence it did earn.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_two_thirds_rejection_rate_raises_the_high_rejection_warning() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("highreject");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    // The check itself passed: nothing violates the contract, and enough
    // cases ran to say so. The warning qualifies that evidence, it does not
    // replace it (D6: statuses sit alongside the evidence order).
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "envelope: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let warnings: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["code"] == "W0503")
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "expected exactly one high-rejection warning: {}",
        run.json
    );
    let title = warnings[0]["title"].as_str().unwrap();
    assert!(
        title.starts_with(
            "most of the inputs generated for `mostly_rejected` were thrown away by its own \
                           `#[ply::requires]` precondition ("
        ),
        "the warning must say what happened in plain words (newbie bar, exact wording): {title}"
    );
    assert!(
        title.contains("draws rejected). proptest kept drawing until it had 256 accepted cases, so the \
                        count is honest"),
        "the warning must not claim fewer cases ran than the verdict says -- proptest draws until it \
         has n accepted cases; what is weak is their spread, not their count: {title}"
    );
    assert!(
        !warnings[0]["fixes"].as_array().unwrap().is_empty(),
        "§8: a non-result diagnostic SHOULD carry concrete fixes: {}",
        warnings[0]
    );
    assert_eq!(
        warnings[0]["severity"], "warning",
        "a weak-evidence warning is not a failure: {}",
        run.json
    );
    assert_eq!(run.exit_code, Some(0), "a warning must not fail the run");
}
