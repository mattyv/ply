//! §5.5's refusal must key on what a callee *is*, never on how the call is
//! spelled (adversarial review of the post-004 fixes, D1, 2026-08-25).
//!
//! `tests/fixtures/unclaimedcallee` writes the call as a bare name that
//! resolves in the caller's own file. This fixture is the same claim over the
//! same unclaimed body, reached through `use rates::{cap_bps as capped,
//! legacy_rate};` -- the most ordinary spelling in Rust. Before the resolver
//! read `use` declarations, that one line converted the refusal into a clean
//! `bounded(2)`, zero diagnostics, exit 0, over a body nobody vouched for:
//! absence of *resolution* silently read as licence to descend.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_use_imported_unclaimed_callee_is_refused_exactly_like_a_qualified_one() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("useimport");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert_eq!(
        verdict, "unclaimed",
        "an unclaimed callee reached through `use` is still an unclaimed callee -- the boundary \
         rule must not be bypassable by spelling the call differently: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "W0512", "envelope: {}", run.json);
    let title = diag["title"].as_str().unwrap();
    assert!(
        title.contains("legacy_rate"),
        "the plainly imported callee must be named: {title}"
    );
    assert!(
        title.contains("capped"),
        "and so must the renamed one, spelled as the caller writes it -- a reader hunting the \
         call site searches for the name in front of them: {title}"
    );

    // The refusal is a call-graph decision: no harness is ever written.
    assert!(
        !fixture.path().join("src/ply_generated.rs").exists(),
        "the refusal must happen before harness codegen, not after a Kani run"
    );

    assert_eq!(
        run.exit_code,
        Some(1),
        "absence of evidence fails the run by default (§1, §6): {}",
        run.json
    );
}
