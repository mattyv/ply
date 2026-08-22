//! Parse-error quality: one diagnostic per root cause, with mechanical fixes for the
//! common classes (§16, M0).

mod util;
use ply_diag::{Code, Diagnostics, SourceMap};

fn diags(src: &str) -> (Diagnostics, SourceMap) {
    let mut sm = SourceMap::new();
    let file = sm.add("test.ply", src);
    let mut d = Diagnostics::new();
    let _ = ply_syntax::parse_file(file, &sm.source(file), &mut d);
    (d.sorted(), sm)
}

/// `(code, first fix title, text after applying that fix)`
fn first(src: &str) -> (Code, Option<String>, Option<String>) {
    let (d, _) = diags(src);
    let first = d.iter().next().expect("expected a diagnostic");
    let fix = first.fixes.first();
    (
        first.code,
        fix.map(|f| f.title.clone()),
        fix.map(|f| ply_diag::apply_fix(src, f)),
    )
}

fn codes(src: &str) -> Vec<Code> {
    diags(src).0.iter().map(|d| d.code).collect()
}

#[test]
fn missing_semicolon_suggests_inserting_one() {
    let src = "fn f() -> Int {\n    let x = 1\n    x\n}\n";
    let (code, title, fixed) = first(src);
    assert_eq!(code, Code::E0110);
    assert_eq!(title.as_deref(), Some("add `;`"));
    assert_eq!(fixed.unwrap(), "fn f() -> Int {\n    let x = 1;\n    x\n}\n");
    assert_eq!(codes(src).len(), 1, "a missing `;` must not cascade");
}

#[test]
fn unclosed_brace_points_at_the_opening_one() {
    let src = "fn f() -> Int {\n    1\n";
    let (d, _) = diags(src);
    let first = d.iter().next().unwrap();
    assert_eq!(first.code, Code::E0111);
    assert_eq!(first.related.len(), 1, "should point at the unclosed `{{`");
    assert_eq!(first.fixes[0].title, "close the `{` here");
    assert_eq!(ply_diag::apply_fix(src, &first.fixes[0]), "fn f() -> Int {\n    1\n}");
}

#[test]
fn unclosed_paren_in_a_call() {
    let src = "fn f() -> Int {\n    g(1, 2\n}\n";
    assert_eq!(codes(src)[0], Code::E0111);
}

#[test]
fn missing_return_type_suggests_unit() {
    let src = "fn f() {\n    ()\n}\n";
    let (code, title, fixed) = first(src);
    assert_eq!(code, Code::E0112);
    assert_eq!(title.as_deref(), Some("return `()`"));
    assert_eq!(fixed.unwrap(), "fn f() -> () {\n    ()\n}\n");
}

#[test]
fn missing_parameter_type_is_e0113() {
    assert_eq!(codes("fn f(x) -> Int { x }")[0], Code::E0113);
}

#[test]
fn ampersand_ampersand_suggests_and() {
    let src = "fn f(a: Bool, b: Bool) -> Bool { a && b }";
    let (code, title, fixed) = first(src);
    assert_eq!(code, Code::E0110);
    assert_eq!(title.as_deref(), Some("use `and`"));
    assert_eq!(fixed.unwrap(), "fn f(a: Bool, b: Bool) -> Bool { a and b }");
}

#[test]
fn pipe_pipe_suggests_or() {
    let src = "fn f(a: Bool, b: Bool) -> Bool { a || b }";
    let (code, title, fixed) = first(src);
    assert_eq!(code, Code::E0110);
    assert_eq!(title.as_deref(), Some("use `or`"));
    assert_eq!(fixed.unwrap(), "fn f(a: Bool, b: Bool) -> Bool { a or b }");
}

#[test]
fn single_equals_in_a_condition_suggests_double() {
    let src = "fn f(a: Int) -> Int { if a = 1 { 1 } else { 0 } }";
    let (code, title, fixed) = first(src);
    assert_eq!(code, Code::E0110);
    assert_eq!(title.as_deref(), Some("use `==`"));
    assert_eq!(fixed.unwrap(), "fn f(a: Int) -> Int { if a == 1 { 1 } else { 0 } }");
}

#[test]
fn expected_expression_is_e0115() {
    assert_eq!(codes("fn f() -> Int { 1 + }")[0], Code::E0115);
}

#[test]
fn one_diagnostic_per_item() {
    // Two independent mistakes in two functions: two diagnostics, not a cascade.
    let src = "fn a() -> Int {\n    let x = 1\n    x\n}\nfn b() -> Int {\n    let y = 2\n    y\n}\n";
    assert_eq!(codes(src), vec![Code::E0110, Code::E0110]);
}

#[test]
fn recovery_keeps_later_items_parseable() {
    let src = "fn a() -> Int { let x = 1 x }\nstruct S { v: Int }\n";
    let (_, sm) = diags(src);
    let mut d = Diagnostics::new();
    let ast = ply_syntax::parse_file(sm.ids().next().unwrap(), &sm.source(sm.ids().next().unwrap()), &mut d);
    assert_eq!(ast.items.len(), 2, "the struct after a broken fn must still parse");
}
