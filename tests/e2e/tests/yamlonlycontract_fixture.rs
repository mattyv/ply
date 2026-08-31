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
/// resolved.
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
            "1 of them also writes a `requires:`/`ensures:` contract directly in ply.yaml -- \
             `verify` does not read a contract written there yet, only \
             `#[ply::requires]`/`#[ply::ensures]` attributes written on the function itself do. \
             Move the contract onto `seven` as an attribute if you want it checked."
        ),
        "check must name the fact plainly, before verify is ever run: {flat}"
    );
}

/// `verify` used to run none of the declared checks and explain that with
/// two diagnostics on `seven` that contradicted each other. Now there is
/// exactly one, and it is honest about what happened and why.
#[test]
fn verify_reports_one_honest_warning_not_two_contradictory_ones() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("yamlonlycontract");
    let run = run_verify(&cargo_ply, fixture.path(), 60);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let on_seven: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["node_id"] == "yamlonlycontract::seven")
        .collect();
    assert_eq!(
        on_seven.len(),
        1,
        "one honest diagnostic, not two contradictory ones: {}",
        run.json
    );
    let title = on_seven[0]["title"].as_str().unwrap();
    assert_eq!(on_seven[0]["code"], "V0505", "{}", run.json);
    assert!(
        !title.contains("checked `seven` against its inline"),
        "must never claim inline attributes were checked when there are none: {title}"
    );
    assert!(
        title.contains("no `#[ply::ensures]`") && title.contains("no `examples:`"),
        "must still say why nothing ran: {title}"
    );
    assert!(
        title.contains("ply.yaml"),
        "must name that ply.yaml also declares a contract here, unread: {title}"
    );
    assert!(
        title.contains("Move the contract onto `seven` as an attribute"),
        "must tell the reader what to do next: {title}"
    );
}
