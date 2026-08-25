//! `cargo ply worklist` end to end (§6). Engine-free by construction, like
//! `check` and `audit`: an open item is something somebody recorded, and
//! reading a record needs no engine.

use ply_e2e::{build_cargo_ply, copy_fixture};

fn run(args: &[&str]) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let out = std::process::Command::new(&cargo_ply)
        .args(args)
        .output()
        .expect("spawning cargo-ply worklist");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// §5.5's honesty condition 3, from the other side: the assumption is
/// permanent trust surface (`audit`), and the evidence owed on it is work
/// that closes (`worklist`). The `boundarycontract` fixture is the case,
/// and the run must say what would close it.
#[test]
fn the_owed_evidence_on_an_assumed_contract_reaches_the_human_surface() {
    let fixture = copy_fixture("boundarycontract");
    let (code, stdout, stderr) = run(&["worklist", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("owed evidence (1)"), "{stdout}");
    assert!(
        stdout.contains("`boundarycontract::tiered_fee`"),
        "{stdout}"
    );
    // Short fragments on purpose: the human surface reflows to ~92
    // columns, so a longer quotation tests the wrap width rather than the
    // sentence.
    assert!(stdout.contains("is never checked is green"), "{stdout}");
    assert!(
        stdout.contains("add `checks: [fuzz(256)]` to its `ply.yaml` entry"),
        "an owed item that does not say how to close it is a nag: {stdout}"
    );
    assert!(
        stdout.contains("What this command did NOT look at:"),
        "{stdout}"
    );
    assert!(stdout.contains("weak specs (W0502)"), "{stdout}");
    assert!(stdout.contains("stale claims (W0302)"), "{stdout}");
}

/// A fixture with nothing owed must read as "nothing is recorded", never
/// as "nothing left to do".
#[test]
fn a_crate_that_owes_nothing_says_so_rather_than_printing_an_empty_list() {
    let fixture = copy_fixture("clamp");
    let (code, stdout, stderr) = run(&["worklist", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("Nothing is owed that Ply can see"),
        "{stdout}"
    );
    assert!(
        stdout.contains("exits 0 whether or not it has items"),
        "{stdout}"
    );
}

/// §8: every command emits the envelope.
#[test]
fn json_is_the_section_8_envelope_with_the_open_items_as_data() {
    let fixture = copy_fixture("boundarycontract");
    let (code, stdout, stderr) = run(&["worklist", fixture.path().to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("no envelope: {e}\n{stdout}"));
    assert_eq!(json["command"], "worklist");
    assert_eq!(json["root"]["verdict"], "unclaimed");
    let open = json["open_items"].as_array().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0]["kind"], "owed_evidence");
    assert_eq!(open[0]["node_id"], "boundarycontract::tiered_fee");
    assert!(
        json["trust_surface"].is_null(),
        "the trust surface belongs to `audit`; a `worklist` envelope carrying one would blur \
         the distinction the two commands exist to draw"
    );
}
