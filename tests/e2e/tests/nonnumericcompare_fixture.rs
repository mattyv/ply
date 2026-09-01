//! Acceptance test for the widening defect found pointing Ply at `semver`
//! (2026-09-01): a promise comparing two non-numeric values with `==`
//! could not compile at all, and because every check in a crate shares one
//! generated harness (The-Ply-Spec.md §5.4c), that one comparison used to
//! turn every *other* function's evidence into a tool error too.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    fn find<'a>(n: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
        if n["id"] == id {
            return Some(n);
        }
        n["children"]
            .as_array()?
            .iter()
            .find_map(|child| find(child, id))
    }
    find(&json["root"], id).unwrap_or_else(|| panic!("no node `{id}` in envelope: {json}"))
}

#[test]
fn a_text_comparison_earns_real_evidence_and_never_blames_its_crate_mates() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nonnumericcompare");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    // The true promise: a real, non-`tool_error` verdict.
    let ok = node(&run.json, "Wrapper::new");
    assert_eq!(
        ok["verdict"], "fuzzed(64)",
        "a promise comparing two `&str` values must actually run and pass, not fail to \
         compile: {}",
        run.json
    );

    // The identical promise, made false: a real violation with a real
    // failing input, not a tool error and not a silent clean pass.
    let broken = node(&run.json, "BrokenWrapper::new");
    assert_eq!(
        broken["verdict"], "violation",
        "the same comparison, made false on purpose, must be caught for real: {}",
        run.json
    );

    // No generated file exists for the fixture's own workspace, but every
    // fn sharing this crate's one generated harness must earn its own
    // verdict, whatever else lives beside it.
    let good = node(&run.json, "good_fn");
    assert_eq!(
        good["verdict"], "fuzzed(64)",
        "good_fn is completely correct and unrelated to every text/option/enum comparison in \
         this crate -- it must be checked for real, whatever else in the crate used to break \
         compilation: {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "nonnumericcompare::good_fn"),
        "good_fn has nothing wrong with it and must carry no diagnostic at all: {}",
        run.json
    );
}

#[test]
fn an_option_comparison_earns_real_evidence_and_bites_on_a_broken_promise() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nonnumericcompare");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let ok = node(&run.json, "identity_opt");
    assert_eq!(
        ok["verdict"], "fuzzed(64)",
        "comparing an `Option` value directly with `==` must actually run: {}",
        run.json
    );

    let broken = node(&run.json, "always_none");
    assert_eq!(
        broken["verdict"], "violation",
        "the same comparison, made false on purpose, must be caught for real with a failing \
         `Some(_)` input: {}",
        run.json
    );
}

#[test]
fn an_enum_variant_comparison_earns_real_evidence_and_bites_on_a_broken_promise() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nonnumericcompare");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let ok = node(&run.json, "always_pos");
    assert_eq!(
        ok["verdict"], "fuzzed(64)",
        "comparing a fieldless enum variant directly with `==` must actually run: {}",
        run.json
    );

    let broken = node(&run.json, "maybe_pos");
    assert_eq!(
        broken["verdict"], "violation",
        "the same comparison, made false on purpose, must be caught for real with a negative \
         input: {}",
        run.json
    );
}

#[test]
fn the_overflow_protection_this_widening_exists_for_still_holds() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nonnumericcompare");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    // `x.saturating_add(1)` at `x = 255` gives 255, not 256 -- the promise
    // `result == x + 1` is genuinely broken there, and checking it
    // exhaustively (`bounded`, so the one bad input among 256 possible
    // `u8` values cannot simply be missed the way 64 random samples might)
    // must report that violation, never panic on the overflowing `+ 1`
    // while evaluating the promise itself.
    let n = node(&run.json, "saturating_bump");
    assert_eq!(
        n["verdict"], "violation",
        "the overflow trap this widening exists to guard against must still report the broken \
         promise, not a tool error from a panicking overflow check: {}",
        run.json
    );
}
