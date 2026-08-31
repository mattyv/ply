//! Defect 2 (2026-08-30, "a documented way of writing contracts is
//! accepted, then silently ignored"): a fn claimed entirely through
//! `ply.yaml`'s `requires:`/`ensures:` -- no `#[ply::requires]`/
//! `#[ply::ensures]` attribute anywhere in the source. `check` resolves the
//! claim fine ("N of N fn claims ... point at a function Ply can find") and
//! used to say nothing more; `verify` then ran none of the declared checks
//! and explained that with two warnings on the same fn that flatly
//! contradicted each other -- one saying the ply.yaml contract "is used ...
//! so this run checked `seven` against its inline attributes only", the
//! other saying "there is nothing to check its result against, so nothing
//! was run". Neither command told the reader the one fact that actually
//! matters: a contract written in ply.yaml is not read for a fn's own
//! checks, only an inline attribute is.
//!
//! A same-day fix (2026-08-30) narrowed the second warning so it never
//! fired unless there *was* an inline attribute -- which reopened the
//! silence for a fn whose `checks:` finds nothing to run at all in a
//! different way (2026-08-31 regression, see `yamlonlycontractexample_fixture`).
//! The real fix is to always say the ply.yaml fact once, worded so it is
//! never false regardless of what else did or did not run -- so `verify`
//! now reports *two* diagnostics for this fixture, and they no longer
//! contradict each other, because neither claims more than it knows.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// The CLI reflows diagnostics to a fixed column width, so where a sentence
/// wraps depends on the tempdir path's length. Collapsing whitespace keeps
/// these assertions exact-string about the words themselves.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

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

/// `check` is the command people run first, and it used to give no hint at
/// all that this claim's contract does nothing. The one sentence a user
/// needs now rides on the same line that already reports the claim as
/// resolved. `seven` has its own `checks:` list here, so this is the "you
/// asked for this to be checked" case, not the boundary-declaration case
/// (see `boundarycontract_check`).
#[test]
fn check_names_the_yaml_only_contract_as_unread_on_the_anchors_line() {
    let fixture = copy_fixture("yamlonlycontract");
    let (code, stdout, stderr) = run(&["check", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let flat = unwrapped(&stdout);
    assert!(
        flat.contains("1 of 1 fn claims in this crate point at a function Ply can find"),
        "the claim must still resolve: {flat}"
    );
    assert!(
        flat.contains(
            "1 of them also writes a `requires:`/`ensures:` contract directly in ply.yaml. A \
             ply.yaml contract is only used one way -- a caller of `seven` may assume it, but it \
             is not added to `seven`'s own checks, so `verify` does not check `seven` against \
             it. Move the contract onto `seven` as a `#[ply::requires]`/`#[ply::ensures]` \
             attribute if you want it checked."
        ),
        "check must name the fact plainly, before verify is ever run: {flat}"
    );
}

/// `verify` used to run none of the declared checks and explain that with
/// two diagnostics on `seven` that contradicted each other. Now there are
/// still two, but they no longer disagree: each says only what it actually
/// knows, and the ply.yaml fact is said exactly once, by the diagnostic
/// whose whole job is saying it.
#[test]
fn verify_reports_two_diagnostics_that_no_longer_contradict_each_other() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("yamlonlycontract");
    let run = run_verify(&cargo_ply, fixture.path(), 60);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let on_seven: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["node_id"] == "yamlonlycontract::seven")
        .collect();
    assert_eq!(on_seven.len(), 2, "envelope: {}", run.json);

    let v0505 = on_seven
        .iter()
        .find(|d| d["code"] == "V0505")
        .unwrap_or_else(|| panic!("expected a V0505 diagnostic: {:#?}", on_seven));
    let title = v0505["title"].as_str().unwrap();
    assert!(
        !title.contains("checked `seven` against its inline"),
        "must never claim inline attributes were checked when there are none: {title}"
    );
    assert!(
        title.contains("no `#[ply::ensures]`") && title.contains("no `examples:`"),
        "must still say why nothing ran: {title}"
    );

    let w0510 = on_seven
        .iter()
        .find(|d| d["code"] == "W0510")
        .unwrap_or_else(|| panic!("expected a W0510 diagnostic: {:#?}", on_seven));
    let title = w0510["title"].as_str().unwrap();
    assert!(
        title.contains("ply.yaml"),
        "must name that ply.yaml also declares a contract here, unread for seven's own checks: \
         {title}"
    );
    assert!(
        !title.contains("checked `seven` against its inline"),
        "must never claim inline attributes were checked when there are none: {title}"
    );
    assert!(
        title.contains("does not check `seven` against it"),
        "must say plainly that nothing checks the ply.yaml contract itself: {title}"
    );
}
