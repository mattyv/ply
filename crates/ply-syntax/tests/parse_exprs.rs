//! Expressions: Pratt precedence, postfix chains, literals (§5.2).

mod util;
use util::{dump_expr, dump_src, expr_errors, parse_ok};

#[test]
fn literals() {
    assert_eq!(dump_expr("1_000"), "1000");
    assert_eq!(dump_expr("true"), "true");
    assert_eq!(dump_expr("false"), "false");
    assert_eq!(dump_expr("()"), "unit");
    assert_eq!(dump_expr(r#""hi\n""#), r#""hi\n""#);
    assert_eq!(dump_expr("x"), "x");
}

#[test]
fn arithmetic_precedence() {
    assert_eq!(dump_expr("a + b * c"), "(+ a (* b c))");
    assert_eq!(dump_expr("a * b + c"), "(+ (* a b) c)");
    assert_eq!(dump_expr("a - b - c"), "(- (- a b) c)");
    assert_eq!(dump_expr("a % b / c"), "(/ (% a b) c)");
}

#[test]
fn boolean_precedence_is_or_then_and_then_equality() {
    assert_eq!(dump_expr("a or b and c"), "(or a (and b c))");
    assert_eq!(dump_expr("a and b == c"), "(and a (== b c))");
    // §5.2 puts equality *below* the relational operators
    assert_eq!(dump_expr("a == b < c"), "(== a (< b c))");
    assert_eq!(dump_expr("a < b + c"), "(< a (+ b c))");
}

#[test]
fn unary_binds_tighter_than_any_binary() {
    assert_eq!(dump_expr("not a and b"), "(and (not a) b)");
    assert_eq!(dump_expr("-a + b"), "(+ (neg a) b)");
    assert_eq!(dump_expr("not a == b"), "(== (not a) b)");
    assert_eq!(dump_expr("-x.f"), "(neg (field x f))");
}

#[test]
fn bang_is_accepted_as_not() {
    assert_eq!(dump_expr("!ok"), "(not ok)");
    assert_eq!(dump_expr("!a != b"), "(!= (not a) b)");
}

#[test]
fn parentheses_regroup_but_are_not_kept_in_the_tree() {
    assert_eq!(dump_expr("(a + b) * c"), "(* (+ a b) c)");
    assert_eq!(dump_expr("((a))"), "a");
}

#[test]
fn comparisons_do_not_chain() {
    assert_eq!(expr_errors("a < b < c"), vec!["E0110"]);
    assert_eq!(expr_errors("a == b != c"), vec!["E0110"]);
    // different precedence levels are fine
    assert!(expr_errors("a == b < c").is_empty());
}

#[test]
fn postfix_chains() {
    assert_eq!(dump_expr("f(1, 2)"), "(call f 1 2)");
    assert_eq!(dump_expr("f()"), "(call f)");
    assert_eq!(dump_expr("x.a.b"), "(field (field x a) b)");
    assert_eq!(dump_expr("xs[0]"), "(index xs 0)");
    assert_eq!(dump_expr("xs[i + 1].name"), "(field (index xs (+ i 1)) name)");
    assert_eq!(dump_expr("x.push(1)"), "(method x push 1)");
    assert_eq!(dump_expr("f(a).g(b).h"), "(field (method (call f a) g b) h)");
}

#[test]
fn explicit_type_arguments_on_calls() {
    assert_eq!(dump_expr("id[Int](3)"), "(call id [Int] 3)");
    assert_eq!(dump_expr("pair[Int, List[Bool]](1, xs)"), "(call pair [Int List[Bool]] 1 xs)");
    // still an index when the brackets are not followed by a call
    assert_eq!(dump_expr("xs[Int]"), "(index xs Int)");
    assert_eq!(dump_expr("x.map[Int](f)"), "(method x map [Int] f)");
}

#[test]
fn list_literals() {
    assert_eq!(dump_expr("[]"), "(list)");
    assert_eq!(dump_expr("[1, 2, 3]"), "(list 1 2 3)");
    assert_eq!(dump_expr("[[1], [2]]"), "(list (list 1) (list 2))");
}

#[test]
fn struct_literals() {
    assert_eq!(dump_expr("Point { x: 1, y: 2 }"), "(struct-lit Point (x 1) (y 2))");
    assert_eq!(dump_expr("Empty { }"), "(struct-lit Empty)");
}

#[test]
fn variant_literals_look_like_calls_until_resolution() {
    assert_eq!(dump_expr("None"), "None");
    assert_eq!(dump_expr("Some(1)"), "(call Some 1)");
}

#[test]
fn if_else_chains() {
    assert_eq!(
        dump_expr("if a { 1 } else if b { 2 } else { 3 }"),
        "(if a (block 1) (if b (block 2) (block 3)))"
    );
    assert_eq!(dump_expr("if a { 1 }"), "(if a (block 1))");
}

#[test]
fn struct_literals_are_not_parsed_in_condition_position() {
    // `if S { .. }` is an if with a block body, not a struct literal
    assert_eq!(dump_expr("if S { 1 }"), "(if S (block 1))");
    // but they are fine inside parentheses or call arguments
    assert_eq!(dump_expr("if f(S { x: 1 }) { 1 }"), "(if (call f (struct-lit S (x 1))) (block 1))");
}

#[test]
fn match_expression() {
    assert_eq!(
        dump_expr("match s { Some(x) => x, None => 0 }"),
        "(match s (arm (variant Some x) x) (arm (variant None) 0))"
    );
}

#[test]
fn blocks_are_expressions() {
    assert_eq!(dump_expr("{ let x = 1; x }"), "(block (let x 1) x)");
    assert_eq!(dump_expr("{ }"), "(block)");
}

#[test]
fn underspecification_expressions() {
    assert_eq!(dump_expr("unresolved #7"), "(unresolved 7)");
    assert_eq!(dump_expr("dontcare"), "dontcare");
    assert_eq!(dump_expr("1 + unresolved #0"), "(+ 1 (unresolved 0))");
}

#[test]
fn string_concatenation_uses_plus() {
    assert_eq!(dump_expr(r#""a" + b"#), r#"(+ "a" b)"#);
}

#[test]
fn expressions_appear_in_item_position_via_bodies() {
    parse_ok("fn f() -> Int { let mut t = 0; t = t + 1; t }");
    assert_eq!(
        dump_src("fn f() -> Int { 1 + 2 }"),
        "(file (fn f [] -> Int (block (+ 1 2))))"
    );
}
