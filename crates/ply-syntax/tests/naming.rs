//! Naming conventions are checked, not merely conventional (§5.1, E0101).

mod util;
use ply_diag::{Code, Diagnostics, SourceMap};

fn check(src: &str) -> Vec<(Code, String, Option<String>)> {
    let mut sm = SourceMap::new();
    let file = sm.add("test.ply", src);
    let mut d = Diagnostics::new();
    let ast = ply_syntax::parse_file(file, &sm.source(file), &mut d);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|x| x.code).collect::<Vec<_>>());
    let mut nd = Diagnostics::new();
    ply_syntax::naming::check_file(&ast, &mut nd);
    nd.sorted()
        .iter()
        .map(|x| {
            (
                x.code,
                sm.snippet(x.primary_span).to_string(),
                x.fixes.first().map(|f| f.edits[0].text.clone()),
            )
        })
        .collect()
}

fn clean(src: &str) {
    assert!(check(src).is_empty(), "expected no naming diagnostics for {src:?}: {:?}", check(src));
}

#[test]
fn conforming_programs_are_silent() {
    clean("struct HttpRequest { url_path: String, retry_count: Int }");
    clean("enum Result2[T, E] { Ok2(T), Err2(E) }");
    clean("fn total_fees(orders: List[Order]) -> Int { 0 }");
    clean("fn f() -> () { let _unused = 1; }");
    clean("use orders;");
    clean("rules AccessControl { rel can_read(String); }");
    clean("machine OrderLifecycle { states Draft -> Placed; }");
}

#[test]
fn functions_must_be_snake_case() {
    assert_eq!(
        check("fn totalFees() -> Int { 0 }"),
        vec![(Code::E0101, "totalFees".into(), Some("total_fees".into()))]
    );
}

#[test]
fn types_must_be_upper_camel() {
    assert_eq!(
        check("struct http_request { x: Int }"),
        vec![(Code::E0101, "http_request".into(), Some("HttpRequest".into()))]
    );
    assert_eq!(
        check("enum shape { Circle }"),
        vec![(Code::E0101, "shape".into(), Some("Shape".into()))]
    );
}

#[test]
fn variants_and_generics_must_be_upper_camel() {
    assert_eq!(
        check("enum E2 { ok_value(Int) }"),
        vec![(Code::E0101, "ok_value".into(), Some("OkValue".into()))]
    );
    assert_eq!(
        check("fn id[t](x: t) -> t { x }"),
        vec![
            (Code::E0101, "t".into(), Some("T".into())),
            (Code::E0101, "t".into(), Some("T".into())),
            (Code::E0101, "t".into(), Some("T".into())),
        ]
    );
}

#[test]
fn fields_params_and_locals_must_be_snake_case() {
    assert_eq!(
        check("struct S { retryCount: Int }"),
        vec![(Code::E0101, "retryCount".into(), Some("retry_count".into()))]
    );
    // Both the binding and every use of it are flagged, so applying the fixes renames
    // the whole program in one pass.
    assert_eq!(
        check("fn f(maxSize: Int) -> Int { maxSize }"),
        vec![
            (Code::E0101, "maxSize".into(), Some("max_size".into())),
            (Code::E0101, "maxSize".into(), Some("max_size".into())),
        ]
    );
    assert_eq!(
        check("fn f() -> Int { let myVar = 1; myVar }"),
        vec![
            (Code::E0101, "myVar".into(), Some("my_var".into())),
            (Code::E0101, "myVar".into(), Some("my_var".into())),
        ]
    );
    assert_eq!(
        check("fn f(xs: List[Int]) -> () { for eachItem in xs { } }"),
        vec![(Code::E0101, "eachItem".into(), Some("each_item".into()))]
    );
}

#[test]
fn machine_states_must_be_upper_camel() {
    assert_eq!(
        check("machine M { states draft -> Placed; }"),
        vec![(Code::E0101, "draft".into(), Some("Draft".into()))]
    );
}

#[test]
fn relations_and_rule_variables_must_be_snake_case() {
    assert_eq!(
        check("rules R { rel Parent(String, String); }"),
        vec![(Code::E0101, "Parent".into(), Some("parent".into()))]
    );
    assert_eq!(
        check("rules R { rel p(Int); q(X) :- p(X); }"),
        vec![
            (Code::E0101, "X".into(), Some("x".into())),
            (Code::E0101, "X".into(), Some("x".into())),
        ]
    );
}

#[test]
fn query_bindings_must_be_snake_case() {
    assert_eq!(
        // Only the binder is flagged: `O` in `select` is a path, and an UpperCamel path
        // could legitimately be a nullary variant until the resolver says otherwise.
        check("fn f(os: List[Order]) -> List[Order] { query { from O in os select O } }"),
        vec![(Code::E0101, "O".into(), Some("o".into()))]
    );
}

#[test]
fn applying_the_fix_makes_the_program_clean() {
    let src = "fn totalFees(maxSize: Int) -> Int { maxSize }";
    let mut sm = SourceMap::new();
    let file = sm.add("test.ply", src);
    let mut d = Diagnostics::new();
    let ast = ply_syntax::parse_file(file, &sm.source(file), &mut d);
    let mut nd = Diagnostics::new();
    ply_syntax::naming::check_file(&ast, &mut nd);
    let mut fixed = src.to_string();
    // Apply back-to-front so earlier spans stay valid.
    let mut all: Vec<_> = nd.sorted().into_vec();
    all.reverse();
    for diag in &all {
        fixed = ply_diag::apply_fix(&fixed, &diag.fixes[0]);
    }
    assert_eq!(fixed, "fn total_fees(max_size: Int) -> Int { max_size }");
    clean(&fixed);
}
