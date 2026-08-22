//! `ply fmt`: one canonical rendering, zero options (§14).

mod util;
use ply_diag::{Diagnostics, SourceMap};
use util::parse_ok;

fn fmt(src: &str) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add("test.ply", src);
    let mut diags = Diagnostics::new();
    let ast = ply_syntax::parse_file(file, &sm.source(file), &mut diags);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics for {src:?}:\n{}",
        ply_diag::render_all(&sm, &diags, ply_diag::Color::Never)
    );
    ply_syntax::format_file(&ast)
}

/// Formatting must be a fixed point and must not change the tree.
#[track_caller]
fn check(src: &str, expected: &str) {
    let out = fmt(src);
    assert_eq!(out, expected, "\n--- got ---\n{out}\n--- want ---\n{expected}");
    assert_eq!(fmt(&out), out, "formatter is not idempotent");
    assert_eq!(
        ply_syntax::dump::dump_file(&parse_ok(src)),
        ply_syntax::dump::dump_file(&parse_ok(&out)),
        "formatting changed the tree"
    );
}

#[test]
fn function_bodies() {
    check("fn  add ( a : Int , b : Int ) ->Int{a+b}", "fn add(a: Int, b: Int) -> Int {\n    a + b\n}\n");
    check("fn f()->(){}", "fn f() -> () {}\n");
    check("fn f()->(){();}", "fn f() -> () {\n    ();\n}\n");
}

#[test]
fn structs_and_enums_always_break() {
    check("struct Point{x:Int,y:Int}", "struct Point {\n    x: Int,\n    y: Int,\n}\n");
    check("struct Empty{}", "struct Empty {}\n");
    check(
        "enum Shape{Circle(Int),Rect(Int,Int),Empty}",
        "enum Shape {\n    Circle(Int),\n    Rect(Int, Int),\n    Empty,\n}\n",
    );
    check("struct P[A,B]{v:A}", "struct P[A, B] {\n    v: A,\n}\n");
}

#[test]
fn signature_clauses_go_on_their_own_lines() {
    check(
        "fn f(a:Int)->Int requires a>0 ensures |r| r>a verify prove {a+1}",
        "fn f(a: Int) -> Int\n    requires a > 0\n    ensures |r| r > a\n    verify prove\n{\n    a + 1\n}\n",
    );
    check(
        "fn p(s:String)->() uses{io.write}{print(s);}",
        "fn p(s: String) -> ()\n    uses { io.write }\n{\n    print(s);\n}\n",
    );
    check("fn f()->Int verify fuzz(runs:10){1}", "fn f() -> Int\n    verify fuzz(runs: 10)\n{\n    1\n}\n");
}

#[test]
fn parentheses_are_re_derived_from_precedence() {
    let wrap = |e: &str| format!("fn f(a: Int, b: Int, c: Int) -> Int {{\n    {e}\n}}\n");
    check("fn f(a:Int,b:Int,c:Int)->Int{(a+b)*c}", &wrap("(a + b) * c"));
    check("fn f(a:Int,b:Int,c:Int)->Int{a+(b*c)}", &wrap("a + b * c"));
    check("fn f(a:Int,b:Int,c:Int)->Int{a-(b-c)}", &wrap("a - (b - c)"));
    check("fn f(a:Int,b:Int,c:Int)->Int{(a-b)-c}", &wrap("a - b - c"));
    check("fn f(a:Int,b:Int,c:Int)->Int{-(a+b)}", &wrap("-(a + b)"));
}

#[test]
fn comparisons_keep_the_parentheses_they_need() {
    // `<` does not chain, so an equal-precedence operand keeps its parentheses...
    check(
        "fn f(a:Int,b:Int,c:Int)->Bool{a<(b<c)}",
        "fn f(a: Int, b: Int, c: Int) -> Bool {\n    a < (b < c)\n}\n",
    );
    // ...while a lower-precedence context does not need them.
    check(
        "fn f(a:Int,b:Int,c:Bool)->Bool{(a<b)==c}",
        "fn f(a: Int, b: Int, c: Bool) -> Bool {\n    a < b == c\n}\n",
    );
}

#[test]
fn struct_literals_in_condition_position_keep_their_parentheses() {
    check(
        "fn f()->Int{if (S{x:1}).x {1} else {0}}",
        "fn f() -> Int {\n    if (S { x: 1 }).x {\n        1\n    } else {\n        0\n    }\n}\n",
    );
    check(
        "fn f()->Int{match (S{x:1}).x { _ => 0 }}",
        "fn f() -> Int {\n    match (S { x: 1 }).x {\n        _ => 0,\n    }\n}\n",
    );
    // Inside a call argument the parser allows them again, so no parentheses are added.
    check(
        "fn f()->Int{if g(S{x:1}) {1} else {0}}",
        "fn f() -> Int {\n    if g(S { x: 1 }) {\n        1\n    } else {\n        0\n    }\n}\n",
    );
}

#[test]
fn bang_is_normalised_to_not() {
    check("fn f(a:Bool)->Bool{!a}", "fn f(a: Bool) -> Bool {\n    not a\n}\n");
    check(
        "fn f(a:Bool,b:Bool)->Bool{!(a and b)}",
        "fn f(a: Bool, b: Bool) -> Bool {\n    not (a and b)\n}\n",
    );
}

#[test]
fn statements() {
    check(
        "fn f()->Int{let mut t=0;while t<10 invariant t>=0 decreases 10-t {t=t+1;} t}",
        "fn f() -> Int {\n    let mut t = 0;\n    while t < 10\n        invariant t >= 0\n        decreases 10 - t\n    {\n        t = t + 1;\n    }\n    t\n}\n",
    );
    check(
        "fn f(xs:List[Int])->(){for x in xs{print2(x);}}",
        "fn f(xs: List[Int]) -> () {\n    for x in xs {\n        print2(x);\n    }\n}\n",
    );
    check("fn f()->Int{return 1;}", "fn f() -> Int {\n    return 1;\n}\n");
    check("fn f(x:Int)->(){let y:Int=x;}", "fn f(x: Int) -> () {\n    let y: Int = x;\n}\n");
}

#[test]
fn match_arms_are_one_per_line_with_trailing_commas() {
    check(
        "fn f(x:Option[Int])->Int{match x{Some(v)=>v,None=>0}}",
        "fn f(x: Option[Int]) -> Int {\n    match x {\n        Some(v) => v,\n        None => 0,\n    }\n}\n",
    );
}

#[test]
fn if_else_chains() {
    check(
        "fn f(a:Int)->Int{if a>0{1}else if a<0{2}else{3}}",
        "fn f(a: Int) -> Int {\n    if a > 0 {\n        1\n    } else if a < 0 {\n        2\n    } else {\n        3\n    }\n}\n",
    );
}

#[test]
fn collections_stay_flat_when_they_fit() {
    check("fn f()->List[Int]{[1,2,3]}", "fn f() -> List[Int] {\n    [1, 2, 3]\n}\n");
    check(
        "fn f()->Point{Point{x:1,y:2}}",
        "fn f() -> Point {\n    Point { x: 1, y: 2 }\n}\n",
    );
}

#[test]
fn long_argument_lists_break_one_per_line() {
    let names: Vec<String> = (0..6).map(|i| format!("argument_number_{i}")).collect();
    let src = format!("fn f()->Int{{combine({})}}", names.join(","));
    let mut want = String::from("fn f() -> Int {\n    combine(\n");
    for n in &names {
        want.push_str(&format!("        {n},\n"));
    }
    want.push_str("    )\n}\n");
    // the flat form would be 103 columns, past the 100-column limit
    check(&src, &want);
}

#[test]
fn long_binary_chains_flatten_and_break_before_each_operator() {
    let a = "a".repeat(32);
    let b = "b".repeat(32);
    let c = "c".repeat(32);
    let src = format!("fn f({a}: Int, {b}: Int, {c}: Int) -> Int {{ {a} + {b} + {c} }}");
    let want = format!(
        "fn f(\n    {a}: Int,\n    {b}: Int,\n    {c}: Int,\n) -> Int {{\n    {a}\n        + {b}\n        + {c}\n}}\n"
    );
    check(&src, &want);
}

#[test]
fn long_parameter_lists_break_one_per_line() {
    let src = "fn combine(alpha_value:Int,beta_value:Int,gamma_value:Int,delta_value:Int,epsilon:Int)->Int{1}";
    check(
        src,
        "fn combine(\n    alpha_value: Int,\n    beta_value: Int,\n    gamma_value: Int,\n    delta_value: Int,\n    epsilon: Int,\n) -> Int {\n    1\n}\n",
    );
}

#[test]
fn items_are_separated_by_one_blank_line() {
    check(
        "fn a()->(){}\nfn b()->(){}\n",
        "fn a() -> () {}\n\nfn b() -> () {}\n",
    );
    check("use a;\nuse b;\n", "use a;\nuse b;\n");
    check("use a;\n\nuse b;\n", "use a;\n\nuse b;\n");
}

#[test]
fn comments_are_preserved() {
    check(
        "// header\nfn f() -> () { // after brace\n    // inner\n    ();\n}\n",
        "// header\nfn f() -> () {\n    // after brace\n    // inner\n    ();\n}\n",
    );
    check(
        "fn f() -> () {\n    (); // trailing\n}\n",
        "fn f() -> () {\n    (); // trailing\n}\n",
    );
    check(
        "struct S {\n    /* doc */\n    x: Int,\n}\n",
        "struct S {\n    /* doc */\n    x: Int,\n}\n",
    );
    check("fn f() -> () {}\n// tail\n", "fn f() -> () {}\n\n// tail\n");
}

#[test]
fn blank_lines_inside_a_body_are_capped_at_one() {
    check(
        "fn f() -> () {\n    ();\n\n\n\n    ();\n}\n",
        "fn f() -> () {\n    ();\n\n    ();\n}\n",
    );
}

#[test]
fn queries() {
    check(
        "fn f(os:List[Order])->List[Order]{query{from o in os select o}}",
        "fn f(os: List[Order]) -> List[Order] {\n    query { from o in os select o }\n}\n",
    );
    let src = "fn f(os:List[Order],cs:List[Customer])->List[Row]{query{from o in os,from c in cs where o.cid==c.id group o by o.cid into g select Row{id:g.key,total:sum(g,.total)} order by g.key desc hint(prefer:hash_join)}}";
    check(
        src,
        "fn f(os: List[Order], cs: List[Customer]) -> List[Row] {\n    query {\n        from o in os,\n        from c in cs\n        where o.cid == c.id\n        group o by o.cid into g\n        select Row { id: g.key, total: sum(g, .total) }\n        order by g.key desc\n        hint(prefer: hash_join)\n    }\n}\n",
    );
}

#[test]
fn rules_blocks() {
    check(
        "rules Access{rel parent(String,String);ancestor(x,y):-parent(x,y);ancestor(x,z):-parent(x,y),ancestor(y,z),not blocked(z);}",
        "rules Access {\n    rel parent(String, String);\n    ancestor(x, y) :- parent(x, y);\n    ancestor(x, z) :- parent(x, y), ancestor(y, z), not blocked(z);\n}\n",
    );
}

#[test]
fn machine_blocks() {
    check(
        "machine Order{states Draft->Placed->Filled|Cancelled; Placed->Cancelled when !ev.partial; invariant: true;}",
        "machine Order {\n    states Draft -> Placed -> Filled | Cancelled;\n    Placed -> Cancelled when not ev.partial;\n    invariant: true;\n}\n",
    );
}

#[test]
fn examples() {
    check("example add(1,2)==3;", "example add(1, 2) == 3;\n");
    // consecutive one-line items stay packed
    check(
        "example a(1)==1;\nexample b(2)==2;\n",
        "example a(1) == 1;\nexample b(2) == 2;\n",
    );
    check(
        "example a(1)==1;\n\nexample b(2)==2;\n",
        "example a(1) == 1;\n\nexample b(2) == 2;\n",
    );
}

#[test]
fn verification_dial_defaults_are_made_explicit() {
    check("fn f()->Int verify bounded{1}", "fn f() -> Int\n    verify bounded(depth: 2)\n{\n    1\n}\n");
    check("fn f()->Int verify induct{1}", "fn f() -> Int\n    verify induct(k: 2)\n{\n    1\n}\n");
    check("fn f()->Int verify fuzz{1}", "fn f() -> Int\n    verify fuzz(runs: 256)\n{\n    1\n}\n");
}

#[test]
fn a_file_header_comment_keeps_its_blank_line() {
    check("// header\n\nuse a;\n", "// header\n\nuse a;\n");
    check("// doc\nuse a;\n", "// doc\nuse a;\n");
}

#[test]
fn string_literals_are_re_escaped_canonically() {
    check(
        "fn f()->String{\"a\\u{41}\\tb\"}",
        "fn f() -> String {\n    \"aA\\tb\"\n}\n",
    );
}

#[test]
fn underspecification_survives_formatting() {
    check(
        "fn f()->Int{ unresolved #3 }",
        "fn f() -> Int {\n    unresolved #3\n}\n",
    );
    check("fn f()->Int ensures |r| r>0 {dontcare}", "fn f() -> Int\n    ensures |r| r > 0\n{\n    dontcare\n}\n");
}
