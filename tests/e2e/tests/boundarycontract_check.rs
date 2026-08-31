//! Defect 2 (2026-08-30, "`check` tells users of the boundary-contract
//! feature to destroy it, with a false sentence"): `legacy_rate` declares a
//! `requires:`/`ensures:` contract in ply.yaml and no `checks:` of its own
//! -- a deliberate boundary declaration (The-Ply-Spec.md §5.5), used so
//! `tiered_fee`'s proof can assume `legacy_rate`'s promise without ever
//! touching `legacy_rate`'s own source. `check` used to say, of this exact
//! fn, "`verify` does not read a contract written there yet" (false --
//! `verify` reads it and uses it, just not as `legacy_rate`'s own check)
//! and "Move the contract onto `legacy_rate` as an attribute if you want
//! it checked" (also wrong: that is advice to delete the very feature this
//! fixture demonstrates, aimed at a fn that never asked to be checked at
//! all). Both sentences are wrong for a boundary-only fn and must read
//! differently than they do for a fn that actually has `checks:` and wants
//! to be verified (see `yamlonlycontract_fixture`'s `check` test).

use ply_e2e::{build_cargo_ply, copy_fixture};

fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn check_never_tells_a_boundary_only_fn_to_move_its_contract() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("boundarycontract");
    let out = std::process::Command::new(&cargo_ply)
        .args(["check", fixture.path().to_str().unwrap()])
        .output()
        .expect("spawning cargo-ply check");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let flat = unwrapped(&stdout);

    assert!(
        out.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !flat.contains("Move the contract onto `legacy_rate`"),
        "must never advise moving a boundary-only fn's contract onto it -- that fn never \
         asked to be checked, and this feature exists so it does not have to be: {flat}"
    );
    assert!(
        !flat.contains("does not read a contract written there yet"),
        "false: `verify` does read a ply.yaml contract, it just does not add it to the fn's \
         own checks: {flat}"
    );
    assert!(
        flat.contains(
            "1 of them, `legacy_rate`, declares a `requires:`/`ensures:` contract in ply.yaml \
             but asks for no checks of its own"
        ),
        "must distinguish the boundary-only case from the wants-to-be-checked case: {flat}"
    );
    assert!(
        flat.contains("any caller's result will say it rests on an unchecked promise"),
        "must say the practical consequence: {flat}"
    );
}
