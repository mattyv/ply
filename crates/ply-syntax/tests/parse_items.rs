//! Items, types and signatures (§5.2, §5.3, §5.4, §5.6).

mod util;
use util::{dump_src, errors, parse_ok};

#[test]
fn use_declaration() {
    assert_eq!(dump_src("use orders;"), "(file (use orders))");
}

#[test]
fn struct_with_fields_and_generics() {
    assert_eq!(
        dump_src("struct Pair[A, B] { left: A, right: List[B] }"),
        "(file (struct Pair [A B] (field left A) (field right List[B])))"
    );
}

#[test]
fn struct_allows_a_trailing_comma() {
    parse_ok("struct P { x: Int, }");
}

#[test]
fn enum_with_payloads() {
    assert_eq!(
        dump_src("enum Shape { Circle(Int), Rect(Int, Int), Empty }"),
        "(file (enum Shape [] (variant Circle Int) (variant Rect Int Int) (variant Empty)))"
    );
}

#[test]
fn fn_signature_with_every_clause() {
    let src = "fn f(a: Int, b: &List[Int], c: &mut Int) -> Bool \
               uses { io.write, db.read } \
               requires a > 0 \
               ensures |r| r == true \
               decreases a \
               verify bounded(depth: 3) { true }";
    assert_eq!(
        dump_src(src),
        "(file (fn f [] (param a own Int) (param b ref List[Int]) (param c refmut Int) \
         -> Bool (uses io.write db.read) (requires (> a 0)) (ensures r (== r true)) \
         (decreases a) (verify bounded(depth: 3)) (block true)))"
    );
}

#[test]
fn fn_without_uses_is_pure() {
    assert_eq!(dump_src("fn g() -> () { () }"), "(file (fn g [] -> unit (block unit)))");
}

#[test]
fn empty_uses_clause_is_allowed() {
    assert_eq!(dump_src("fn g() -> () uses { } { () }"), "(file (fn g [] -> unit (uses) (block unit)))");
}

#[test]
fn all_verify_modes_parse_with_and_without_arguments() {
    for (src, expect) in [
        ("verify test", "test"),
        ("verify fuzz", "fuzz(runs: 256)"),
        ("verify fuzz(runs: 10)", "fuzz(runs: 10)"),
        ("verify bounded", "bounded(depth: 2)"),
        ("verify bounded(depth: 7)", "bounded(depth: 7)"),
        ("verify induct", "induct(k: 2)"),
        ("verify induct(k: 4)", "induct(k: 4)"),
        ("verify prove", "prove"),
    ] {
        let text = format!("fn g() -> () {src} {{ () }}");
        assert!(
            dump_src(&text).contains(&format!("(verify {expect})")),
            "{src} produced {}",
            dump_src(&text)
        );
    }
}

#[test]
fn unknown_verify_mode_is_e0118() {
    assert_eq!(errors("fn g() -> () verify wobble { () }"), vec!["E0118"]);
}

#[test]
fn unknown_capability_is_e0119() {
    assert_eq!(errors("fn g() -> () uses { net.connect } { () }"), vec!["E0119"]);
}

#[test]
fn generic_function() {
    assert_eq!(
        dump_src("fn id[T](x: T) -> T { x }"),
        "(file (fn id [T] (param x own T) -> T (block x)))"
    );
}

#[test]
fn example_declaration() {
    assert_eq!(dump_src("example add(1, 2) == 3;"), "(file (example add (1 2) 3))");
}

#[test]
fn multiple_items_in_one_file() {
    let src = "use a; struct S { x: Int } fn f() -> () { () }";
    assert_eq!(
        dump_src(src),
        "(file (use a) (struct S [] (field x Int)) (fn f [] -> unit (block unit)))"
    );
}

#[test]
fn borrow_annotation_outside_a_parameter_is_e0123() {
    assert_eq!(errors("struct S { x: &Int }"), vec!["E0123"]);
    assert_eq!(errors("fn f() -> &Int { 1 }"), vec!["E0123"]);
}

#[test]
fn item_keyword_expected_at_top_level() {
    assert_eq!(errors("fn f() -> () { () } wat"), vec!["E0116"]);
}
