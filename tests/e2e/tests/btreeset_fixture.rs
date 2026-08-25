//! M4 acceptance -- the point of the whole milestone: a shape §5.4b
//! excludes from `bounded` (`BTreeSet`, measured intractable for Kani past
//! one element) earns an honest `fuzzed(n)` verdict. This fixture declares
//! no `checks:` at all, so it also exercises the shape-aware default
//! routing (§5.4c): `[fuzz(256)]` because the shape fails the Kani gate but
//! passes the fuzz gate, never `[bounded(2)]` and never silently nothing.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn kani_excluded_btreeset_shape_earns_an_honest_fuzzed_verdict_via_the_default_route() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("btreeset");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "envelope: {}",
        run.json
    );
    assert_eq!(
        run.json["diagnostics"].as_array().unwrap().len(),
        0,
        "envelope: {}",
        run.json
    );

    // No Kani harness was ever generated: this fn never entered the
    // bounded/Kani path at all.
    assert!(!fixture.path().join("src/ply_generated.rs").exists());
    // The fuzz harness crate was, though.
    assert!(fixture.path().join("target/ply/fuzz").is_dir());
}
