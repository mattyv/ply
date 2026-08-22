//! Statements, loop annotations and patterns (§5.2).

mod util;
use util::{dump_expr, errors, parse_ok};

fn body(src: &str) -> String {
    dump_expr(&format!("{{ {src} }}"))
}

#[test]
fn let_bindings() {
    assert_eq!(body("let x = 1;"), "(block (let x 1))");
    assert_eq!(body("let mut x = 1;"), "(block (let mut x 1))");
    assert_eq!(body("let x: Int = 1;"), "(block (let x : Int 1))");
    assert_eq!(body("let mut xs: List[Int] = [];"), "(block (let mut xs : List[Int] (list)))");
}

#[test]
fn assignment_to_places() {
    assert_eq!(body("x = 1;"), "(block (assign x 1))");
    assert_eq!(body("x.a = 1;"), "(block (assign (field x a) 1))");
    assert_eq!(body("xs[0] = 1;"), "(block (assign (index xs 0) 1))");
    assert_eq!(body("x.a[i].b = 1;"), "(block (assign (field (index (field x a) i) b) 1))");
}

#[test]
fn assignment_to_a_non_place_is_e0114() {
    assert_eq!(errors("fn f() -> () { f() = 1; }"), vec!["E0114"]);
    assert_eq!(errors("fn f() -> () { 1 + 2 = 1; }"), vec!["E0114"]);
}

#[test]
fn while_with_loop_annotations() {
    assert_eq!(
        body("while i < n invariant i >= 0 invariant s >= 0 decreases n - i { i = i + 1; }"),
        "(block (while (< i n) (invariant (>= i 0)) (invariant (>= s 0)) (decreases (- n i)) \
         (block (assign i (+ i 1)))))"
    );
    assert_eq!(body("while c { }"), "(block (while c (block)))");
}

#[test]
fn for_over_a_list() {
    assert_eq!(body("for x in xs { }"), "(block (for x xs (block)))");
}

#[test]
fn return_statements() {
    assert_eq!(body("return;"), "(block (return))");
    assert_eq!(body("return 1 + 2;"), "(block (return (+ 1 2)))");
}

#[test]
fn expression_statements_and_the_tail_expression() {
    assert_eq!(body("f(); g()"), "(block (expr (call f)) (call g))");
    assert_eq!(body("f();"), "(block (expr (call f)))");
}

#[test]
fn block_like_expressions_are_statements_without_a_semicolon() {
    assert_eq!(
        body("if c { f(); } g();"),
        "(block (expr (if c (block (expr (call f))))) (expr (call g)))"
    );
    assert_eq!(
        body("match x { _ => f() } g();"),
        "(block (expr (match x (arm _ (call f)))) (expr (call g)))"
    );
}

#[test]
fn patterns() {
    assert_eq!(body("match x { _ => 0 }"), "(block (match x (arm _ 0)))");
    assert_eq!(body("match x { y => y }"), "(block (match x (arm y y)))");
    assert_eq!(body("match x { 1 => 0 }"), "(block (match x (arm 1 0)))");
    assert_eq!(body(r#"match x { "a" => 0 }"#), r#"(block (match x (arm "a" 0)))"#);
    assert_eq!(body("match x { true => 0 }"), "(block (match x (arm true 0)))");
    assert_eq!(body("match x { -1 => 0 }"), "(block (match x (arm -1 0)))");
    assert_eq!(
        body("match x { Ok(Some(v)) => v }"),
        "(block (match x (arm (variant Ok (variant Some v)) v)))"
    );
    assert_eq!(
        body("match p { { x, y: q } => q }"),
        "(block (match p (arm (pat-struct (x) (y q)) q)))"
    );
}

#[test]
fn match_arms_accept_optional_trailing_commas() {
    parse_ok("fn f(x: Int) -> Int { match x { 1 => 1, _ => 0, } }");
    parse_ok("fn f(x: Int) -> Int { match x { 1 => 1, _ => 0 } }");
}

#[test]
fn nested_blocks() {
    // a trailing block-like expression is the block value, not a statement
    assert_eq!(body("{ let x = 1; }"), "(block (block (let x 1)))");
}
