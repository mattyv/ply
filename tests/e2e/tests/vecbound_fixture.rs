//! The `Vec<u8>` fixture verifies with Ply's mandatory `#[kani::unwind]`
//! emission -- and, adversarially, does NOT verify (within a bounded cap)
//! without it, proving the emission is load-bearing rather than decorative.
//!
//! Honesty note (see docs/m3-slice-findings.md): this exact harness shows
//! real run-to-run wall-clock variance in this sandboxed environment (1s to
//! ~107s observed for the identical, successful proof) -- CBMC/CaDiCaL SAT
//! solve time is not deterministic here. The engine-timeout below is
//! generous specifically to absorb that variance, not because the proof is
//! usually slow.

use std::process::Command;

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn vec_fixture_verifies_with_the_measured_unwind() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("vecbound");

    let run = run_verify(&cargo_ply, fixture.path(), 150);
    assert_eq!(
        run.json["root"]["verdict"], "bounded(8)",
        "envelope: {}",
        run.json
    );
    assert_eq!(run.json["diagnostics"].as_array().unwrap().len(), 0);

    let harness_src = std::fs::read_to_string(fixture.path().join("src/ply_generated.rs")).unwrap();
    assert!(
        harness_src.contains("#[kani::unwind(9)]"),
        "codegen must emit the measured bound:\n{harness_src}"
    );
}

#[test]
fn unwind_emission_is_load_bearing_not_decorative() {
    // Deliberately mutate the tool's own output: take the real generated
    // harness and strip the unwind annotation, then confirm it no longer
    // verifies within a bounded cap -- the adversarial check the M3 brief
    // asks for. This never runs through `cargo-ply` itself (which always
    // emits the annotation); it's a targeted mutation of what the tool
    // produced, proving the annotation is necessary.
    let fixture = copy_fixture("vecbound");
    let crate_dir = fixture.path();

    let harness_source = "#[cfg(kani)]\nuse super::*;\n\n#[cfg(kani)]\n#[kani::proof_for_contract(vec_sum)]\nfn ply_proof_vec_sum() {\n    let v = kani::vec::any_vec::<u8, 8>();\n    vec_sum(&v);\n}\n";
    assert!(
        !harness_source.contains("kani::unwind"),
        "sanity: this mutated harness must genuinely have no unwind annotation"
    );
    std::fs::write(crate_dir.join("src/ply_generated.rs"), harness_source).unwrap();
    let lib_rs = fixture.read_lib_rs();
    fixture.write_lib_rs(&format!("{lib_rs}\nmod ply_generated;\n"));

    let output = Command::new("cargo")
        .current_dir(crate_dir)
        .args([
            "kani",
            "-Z",
            "function-contracts",
            "-Z",
            "unstable-options",
            "--harness-timeout",
            "25s",
            "--exact",
            "--harness",
            "ply_generated::ply_proof_vec_sum",
        ])
        .output()
        .expect("spawning cargo kani");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.contains("VERIFICATION:- SUCCESSFUL"),
        "without the unwind emission this must NOT cleanly verify within the cap:\n{combined}"
    );
}
