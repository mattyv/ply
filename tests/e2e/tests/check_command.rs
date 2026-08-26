//! `cargo ply check` end to end (§6). Engine-free by construction: if any of
//! these needs Kani, proptest or cargo-mutants to pass, the command is not
//! the one §6 describes as "fast, no engines".

use ply_e2e::{build_cargo_ply, copy_fixture};

/// The CLI reflows diagnostics to ~92 columns, so where a sentence breaks
/// depends on how long the tempdir path happens to be. Collapsing runs of
/// whitespace keeps these assertions exact-string about the *words* -- which
/// is what §8 and the newbie bar are about -- without coupling them to the
/// wrap column.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn run(args: &[&str]) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let out = std::process::Command::new(&cargo_ply)
        .args(args)
        .output()
        .expect("spawning cargo-ply check");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The whole point of the coverage block: a green run must not read as
/// "your workspace is verified". It has to name the tiers §6 promises and
/// this build does not deliver.
#[test]
fn a_clean_run_exits_zero_and_still_says_what_it_did_not_check() {
    let fixture = copy_fixture("clamp");
    let (code, stdout, stderr) = run(&["check", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("No problems found in the document."),
        "{stdout}"
    );
    assert!(
        stdout.contains("What this command did NOT check:"),
        "{stdout}"
    );
    assert!(stdout.contains("architecture"), "{stdout}");
    assert!(
        !stdout.contains("staleness"),
        "there is no staleness tier to report: a recorded result is settled by re-hashing it \
         when `verify` uses it, so no command has a \"might be out of date\" state to warn \
         about: {stdout}"
    );
    assert!(
        stdout.contains("runs no engines, so it produces no verdicts"),
        "a command that checked nothing about the code must not read like one that did: \
         {stdout}"
    );
}

/// §8: every command emits the envelope, and `--json` is the agent surface.
#[test]
fn json_is_the_section_8_envelope_with_the_coverage_block() {
    let fixture = copy_fixture("clamp");
    let (code, stdout, stderr) = run(&["check", fixture.path().to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("no envelope: {e}\n{stdout}"));
    assert_eq!(json["command"], "check");
    assert!(json["ply_version"].is_string());
    assert_eq!(json["root"]["kind"], "workspace");
    assert_eq!(json["diagnostics"].as_array().unwrap().len(), 0);
    let not_checked = json["coverage"]["not_checked"].as_array().unwrap();
    let tiers: Vec<&str> = not_checked
        .iter()
        .map(|t| t["tier"].as_str().unwrap())
        .collect();
    assert_eq!(tiers, ["architecture"]);
}

/// §5.2's MUST, through the real binary: a renamed function breaks the
/// build rather than quietly orphaning its claim.
#[test]
fn a_renamed_function_fails_the_run_and_the_diagnostic_names_the_near_miss() {
    let fixture = copy_fixture("clamp");
    let src = fixture.read_lib_rs();
    fixture.write_lib_rs(&src.replace("pub fn clamp(", "pub fn clamp_to_max("));

    let (code, stdout, stderr) = run(&["check", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("E0301"), "{stdout}");
    assert!(
        unwrapped(&stdout).contains("The closest name Ply can see is `clamp_to_max`"),
        "{stdout}"
    );
}

/// A schema violation goes through the same surface, carrying the JSON
/// pointer §5 asks for.
#[test]
fn a_typoed_key_is_a_schema_finding_with_its_json_pointer() {
    let fixture = copy_fixture("clamp");
    let yaml = fixture.path().join("ply.yaml");
    let text = std::fs::read_to_string(&yaml).unwrap();
    std::fs::write(
        &yaml,
        text.replace(
            "        checks: [bounded(2)]",
            "        ensure:\n          - \"|result| *result <= x\"",
        ),
    )
    .unwrap();

    let (code, stdout, stderr) = run(&["check", fixture.path().to_str().unwrap(), "--json"]);
    assert_eq!(code, 1, "{stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let d = &json["diagnostics"][0];
    assert_eq!(d["code"], "E0204");
    assert_eq!(d["pointer"], "/components/clamp/fns/clamp/ensure");
    assert!(
        d["title"]
            .as_str()
            .unwrap()
            .contains("Did you mean `ensures`?"),
        "{d}"
    );
}

/// The anchor tier resolves a claim the same way call classification
/// resolves a call, so `check` and `verify` cannot disagree about where a
/// function lives. Before 2026-08-25 this printed `E0301` for a claim on
/// `rates::legacy_rate` while `verify` was, in the same breath, naming
/// `legacy_rate` as a callee nobody had vouched for.
#[test]
fn a_claim_on_a_function_inside_a_module_resolves() {
    let fixture = copy_fixture("modanchor");
    let (code, stdout, stderr) = run(&["check", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(!stdout.contains("E0301"), "{stdout}");
    assert!(
        unwrapped(&stdout)
            .contains("2 of 2 fn claims in this crate point at a function Ply can find"),
        "{stdout}"
    );
}

/// The case that genuinely stays closed, reported as itself. Ply generates
/// its harness at the crate root, so a private item inside a module is a
/// name that harness cannot write -- which is a different fact from "no
/// such function", and gets a different sentence.
#[test]
fn a_private_function_inside_a_module_is_refused_with_the_real_reason() {
    let fixture = copy_fixture("modanchor");
    let rates = fixture.path().join("src/rates.rs");
    let src = std::fs::read_to_string(&rates).unwrap();
    std::fs::write(&rates, src.replace("pub fn legacy_rate", "fn legacy_rate")).unwrap();

    let (code, stdout, stderr) = run(&["check", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("E0301"), "{stdout}");
    assert!(
        stdout.contains("private"),
        "the diagnostic must name the actual obstacle: {stdout}"
    );
    assert!(
        !unwrapped(&stdout).contains("could not find a function called"),
        "the function was found; saying otherwise sends the reader hunting for a typo: {stdout}"
    );
}
