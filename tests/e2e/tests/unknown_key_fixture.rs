//! §5.1a rule 1 on the product path: "a typo must be caught, never ignored".
//!
//! `ply-check` has always enforced `additionalProperties: false` on a
//! `ply.yaml`; `cargo ply verify` read the same document with plain serde and
//! silently dropped every key its own structs did not name. That is how
//! vetting 004's finding 7 happened -- a team writing an external `ensures:`
//! for a legacy callee got no contract and no warning, from the very file the
//! other tool validates. Two tools disagreeing about one document.

use ply_e2e::{build_cargo_ply, copy_fixture};

#[test]
fn a_typo_in_a_ply_yaml_key_is_refused_by_the_verify_path_too() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("clamp");
    let yaml = fixture.path().join("ply.yaml");
    let text = std::fs::read_to_string(&yaml).unwrap();
    // One character off the key that matters most: the contract clause.
    std::fs::write(
        &yaml,
        text.replace(
            "        checks: [bounded(2)]",
            "        ensure:\n          - \"|result| *result <= x\"",
        ),
    )
    .unwrap();

    let output = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap(), "--json"])
        .output()
        .expect("spawning cargo-ply verify");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("E0204"),
        "an unknown key must be refused with E0204, not silently dropped: {stderr}"
    );
    assert!(
        stderr.contains("Did you mean `ensures`?"),
        "and the message must name the nearest key it knows: {stderr}"
    );
    assert!(
        stderr.contains("components.clamp.fns.clamp.ensure"),
        "and say where the key is: {stderr}"
    );
    assert_ne!(output.status.code(), Some(0), "{stderr}");
}

/// The other half of the same rule: `verify` must accept the whole §5
/// grammar, not just the subset it acts on. One document is read by three
/// tools -- vetting 004's own ply.yaml carries `pure:` on a component and
/// `edges:` at the top level, and `verify` ignores both. Rejecting them
/// would break the single-document story this project is built on.
#[test]
fn the_keys_verify_ignores_are_still_accepted() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("clamp");
    let yaml = fixture.path().join("ply.yaml");
    let text = std::fs::read_to_string(&yaml).unwrap();
    std::fs::write(
        &yaml,
        format!(
            "{}\nedges: []\ndeny: []\nprofiles: {{}}\n",
            text.replace(
                "    anchor: ply_fixture_clamp",
                "    anchor: ply_fixture_clamp\n    pure: true\n    strict: false",
            )
        ),
    )
    .unwrap();

    let output = std::process::Command::new(&cargo_ply)
        .args([
            "verify",
            fixture.path().to_str().unwrap(),
            "--json",
            "--engine-timeout",
            "120",
        ])
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "no envelope: {e}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    // The `clamp` fixture's contract is deliberately falsifiable, so the
    // verdict here is `violation` -- what matters is that a run happened at
    // all and no key was refused.
    assert_eq!(json["root"]["verdict"], "violation", "envelope: {json}");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("E0204"),
        "no §5 key may be refused: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
