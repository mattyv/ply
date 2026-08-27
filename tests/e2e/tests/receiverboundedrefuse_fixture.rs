//! "A proof refused on a method blames the u32 return type instead of the
//! receiver" (task 2026-08-27, docs/review-strings-receivers.md). Before
//! this fix, `bounded_refused_sample_only_diag` assumed anything reaching
//! it was refused for a *type* reason, and a receiver method with an
//! otherwise-fine parameter list and return type fell through to blaming
//! the return type by name -- `u32` here, the cheapest type Kani handles.
//! The real, and only, reason `bounded` refuses `Gauge::level` is that it
//! needs a receiver, which the exhaustive tier does not build.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_bounded_refusal_on_a_receiver_method_blames_the_receiver_not_its_return_type() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverboundedrefuse");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_eq!(
        run.json["root"]["verdict"], "unsupported",
        "`bounded` cannot check a receiver method today: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["node_id"] == "receiverboundedrefuse::Gauge::level" && d["code"] == "V0508")
        .unwrap_or_else(|| panic!("no V0508 diagnostic: {}", run.json));
    let title = d["title"].as_str().unwrap();

    assert!(
        !title.contains("its return type `u32`"),
        "the return type is not the problem -- `u32` is one of the cheapest types Kani handles, \
         and blaming it hides the real blocker: {title}"
    );
    assert!(
        title.contains("needs a value to call it on") || title.contains("receiver"),
        "the diagnostic must name the real reason: `bounded` has no receiver construction: \
         {title}"
    );
    assert!(
        title.contains("fuzz"),
        "the diagnostic must point at the check that actually can check this function: {title}"
    );
}
