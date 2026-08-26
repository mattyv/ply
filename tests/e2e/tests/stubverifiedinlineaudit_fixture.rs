//! D4 (adversarial review, 2026-08-26): `audit`'s and `worklist`'s trust
//! surface used to read only the `ply.yaml`-declared boundary-contract
//! route (D5's second branch reached through a declared contract) --
//! never a same-crate callee reached through its *own* inline
//! `#[ply::requires]`/`#[ply::ensures]` (D5's first two branches, §5.5).
//! `verify` already reported `f` here as `conditional`/`owed-evidence`
//! standing on `g`'s assumed inline contract, but `audit` listed no trust
//! surface for it and `worklist` listed no owed evidence for it -- §5.5's
//! own honesty condition 3 ("trust that is never checked is green paint
//! ... `cargo ply audit` lists it") silently did not hold for this class.
//! Engine-free by construction, like the other command tests: this is a
//! call-graph and document read, no Kani involved.

use ply_e2e::{build_cargo_ply, copy_fixture};

fn run(args: &[&str]) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let out = std::process::Command::new(&cargo_ply)
        .args(args)
        .output()
        .expect("spawning cargo-ply");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn an_inline_contracted_callees_assumption_reaches_audits_trust_surface() {
    let fixture = copy_fixture("stubverifiedinlineaudit");
    let (code, stdout, stderr) = run(&["audit", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("assumed contracts (1)"),
        "g's own inline contract is exactly what f's claim stands on -- audit's whole point is \
         to list it, the same as a ply.yaml-declared one: {stdout}"
    );
    assert!(
        stdout.contains('g') && stdout.contains('f'),
        "the callee and the caller standing on it must both be named: {stdout}"
    );
    assert!(
        stdout.contains("ensures |result| *result == x + 1") || stdout.contains("x + 1"),
        "the promise itself has to be readable, or nobody can judge it: {stdout}"
    );
}

#[test]
fn the_same_assumption_reaches_worklists_owed_evidence() {
    let fixture = copy_fixture("stubverifiedinlineaudit");
    let (code, stdout, stderr) = run(&["worklist", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("owed evidence (1)"),
        "an inline-contracted callee that is never independently claimed with a bounded check \
         owes evidence exactly like a ply.yaml-declared one does: {stdout}"
    );
    assert!(stdout.contains("stubverifiedinlineaudit::f"), "{stdout}");
}
