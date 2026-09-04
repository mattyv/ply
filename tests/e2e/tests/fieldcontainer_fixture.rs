//! Container resolution below the top level (TODO.md, "finish the
//! container fix", 2026-09-04). A top-level `Vec<Doc>` parameter already
//! composed (`compositionbites_fixture.rs`); this fixture is the shape one
//! level deeper -- a struct field, an enum variant field, and a
//! constructor argument, each themselves a container of a user type. Each
//! promise here is genuinely false, so this test proves a real run finds
//! each one with a real failing input, and -- the point of building a real
//! crate rather than asserting on generated text -- that the harness Ply
//! writes for this shape actually *compiles*. A per-field/per-argument
//! resolution bug alone would report `unsupported`; a codegen bug in how
//! the raw sampled shape is converted back into the real value would
//! report `tool_error` quoting a compiler error instead of a verdict.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn find_fn_node<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|c| find_fn_node(c, id))
}

#[test]
fn a_false_promise_over_a_struct_field_that_is_a_container_of_a_user_type_earns_a_real_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("fieldcontainer");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let node = find_fn_node(&run.json["root"], "bag_total")
        .unwrap_or_else(|| panic!("no node for bag_total: {}", run.json));
    assert_eq!(
        node["verdict"], "violation",
        "a struct field that is a container of a user type is buildable now -- the promise \
         must actually be checked, and it is false: {}",
        run.json
    );
}

#[test]
fn a_false_promise_over_a_constructor_argument_that_is_a_container_of_a_user_type_earns_a_real_violation(
) {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("fieldcontainer");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let node = find_fn_node(&run.json["root"], "basket_total")
        .unwrap_or_else(|| panic!("no node for basket_total: {}", run.json));
    assert_eq!(
        node["verdict"], "violation",
        "a constructor argument that is a container of a user type is buildable now -- the \
         promise must actually be checked, and it is false: {}",
        run.json
    );
}

#[test]
fn a_false_promise_over_an_enum_variant_field_that_is_a_container_of_a_user_type_earns_a_real_violation(
) {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("fieldcontainer");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let node = find_fn_node(&run.json["root"], "holder_total")
        .unwrap_or_else(|| panic!("no node for holder_total: {}", run.json));
    assert_eq!(
        node["verdict"], "violation",
        "an enum variant field that is a container of a user type is buildable now -- the \
         promise must actually be checked, and it is false: {}",
        run.json
    );
}
