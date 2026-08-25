//! `cargo ply audit` end to end (§6). Engine-free by construction: this
//! runs against the `boundarycontract` fixture, whose whole point is D5's
//! second branch, and it must produce the trust surface without Kani,
//! proptest or cargo-mutants ever starting.

use ply_e2e::{build_cargo_ply, copy_fixture};

fn run(args: &[&str]) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let out = std::process::Command::new(&cargo_ply)
        .args(args)
        .output()
        .expect("spawning cargo-ply audit");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The fixture's `tiered_fee` is proved against a promise made for
/// `legacy_rate` in ply.yaml. Before this command existed, the
/// `owed-evidence` that assumption carries was recorded nowhere and
/// displayed nowhere — §5.5 described `audit` listing it in the present
/// tense while nothing listed anything.
#[test]
fn the_assumed_contract_and_its_owed_evidence_reach_the_human_surface() {
    let fixture = copy_fixture("boundarycontract");
    let (code, stdout, stderr) = run(&["audit", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("assumed contracts (1)"),
        "the tier heading is how a reader finds it: {stdout}"
    );
    assert!(
        stdout.contains("`legacy_rate` — assumed by `boundarycontract::tiered_fee`"),
        "{stdout}"
    );
    assert!(stdout.contains("[owed-evidence]"), "{stdout}");
    assert!(
        stdout.contains("ensures |result| *result <= 10_000"),
        "the promise itself has to be readable, or nobody can judge it: {stdout}"
    );
    assert!(
        stdout.contains("What this command did NOT look at:"),
        "{stdout}"
    );
    assert!(
        stdout.contains("runs no engines, so it produces no verdicts"),
        "{stdout}"
    );
}

/// §8: every command emits the envelope, and `--json` is the agent surface.
#[test]
fn json_is_the_section_8_envelope_with_the_trust_surface_as_data() {
    let fixture = copy_fixture("boundarycontract");
    let (code, stdout, stderr) = run(&["audit", fixture.path().to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("no envelope: {e}\n{stdout}"));
    assert_eq!(json["command"], "audit");
    assert_eq!(json["root"]["verdict"], "unclaimed");
    assert_eq!(json["diagnostics"].as_array().unwrap().len(), 0);
    let surface = json["trust_surface"].as_array().unwrap();
    assert_eq!(surface.len(), 1);
    assert_eq!(surface[0]["kind"], "assumed_contract");
    assert_eq!(surface[0]["subject"], "legacy_rate");
    assert_eq!(surface[0]["statuses"][0], "owed-evidence");
    let not_checked: Vec<&str> = json["coverage"]["not_checked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["tier"].as_str().unwrap())
        .collect();
    assert_eq!(
        not_checked,
        [
            "attestation coverage",
            "assumption discharge",
            "helper evidence",
            "unreadable call sites",
            "architecture bans"
        ]
    );
}

/// A codebase with a trust surface is a normal codebase (§5.5:
/// `conditional` "is the *normal* state of a legacy-extension codebase").
/// The `unclaimedcallee` fixture has no declared contract at all, so it has
/// nothing to trust — and the empty report must read as "nothing is
/// declared here", never as "verified".
#[test]
fn a_crate_that_trusts_nothing_says_so_rather_than_printing_an_empty_list() {
    let fixture = copy_fixture("unclaimedcallee");
    let (code, stdout, stderr) = run(&["audit", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("Nothing in this crate rests on trust that Ply can see"),
        "{stdout}"
    );
    // A short fragment on purpose: the human surface reflows to ~92
    // columns, so a longer quotation would be testing the wrap width
    // rather than the sentence.
    assert!(stdout.contains("not a verdict about the code"), "{stdout}");
}
