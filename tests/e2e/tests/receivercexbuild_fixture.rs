//! Blocker 3 (task 2026-08-27, docs/review-strings-receivers.md finding 3):
//! "Ply breaks the user's own build." When a receiver method breaks its
//! promise, Ply used to render a replay test into the user's own `src/`
//! calling the method with no receiver at all (`Gauge::level()` instead of
//! `Gauge::level(&receiver)`), which does not compile -- and it had already
//! added a `mod` line to `lib.rs` pointing at the broken file, so the user's
//! own `cargo test` stopped building. Ply must never leave a user's crate
//! unbuildable: refuse to render the test, or render one that actually
//! compiles. This pins the refusal, and proves the crate stays buildable by
//! actually running `cargo test --lib` against it afterwards -- not just
//! reading Ply's own report.

use std::process::Command;

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_broken_receiver_method_never_leaves_the_users_crate_unbuildable() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receivercexbuild");

    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert_eq!(
        verdict, "violation",
        "`Gauge::level`'s promise is false on every input -- `fuzz(64)` must catch it: {}",
        run.json
    );

    // Ply must not have added a replay-test module line to the user's own
    // lib.rs when it cannot render a correct one for this shape.
    let lib_rs = fixture.read_lib_rs();
    assert!(
        !lib_rs.contains("mod ply_generated_cex"),
        "no replay test module should be wired into the user's own lib.rs for a receiver method \
         Ply cannot render a receiver value for -- refusing must mean refusing, not partially \
         wiring in a file that cannot compile:\n{lib_rs}"
    );
    let cex_path = fixture.path().join("src/ply_generated_cex.rs");
    assert!(
        !cex_path.exists(),
        "no replay test file should be written at all for this shape: {}",
        cex_path.display()
    );

    // The decisive proof, run for real rather than just asserted: the
    // user's own crate must still build and its own (empty) test suite
    // must still run clean after `cargo ply verify` has done everything it
    // is going to do.
    let status = Command::new("cargo")
        .args(["test", "--lib"])
        .current_dir(fixture.path())
        .status()
        .expect("spawning `cargo test --lib` in the fixture crate");
    assert!(
        status.success(),
        "the user's own crate must still compile and its own tests must still run after `cargo \
         ply verify` -- Ply must never leave a user's crate unbuildable"
    );

    // The diagnostic must still say something honest: no witness silently
    // dropped, no fabricated input, no false "failed to compile" claim
    // about a harness that built fine.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["node_id"] == "receivercexbuild::Gauge::level" && d["code"] == "W0541")
        .unwrap_or_else(|| panic!("no W0541 witness-only diagnostic: {}", run.json));
    assert!(
        d["counterexample"]["kani_witness"].is_null()
            && d["counterexample"]["cargo_test"].is_null(),
        "a refused render must carry no test-file pointer and no replay claim: {d}"
    );
}
