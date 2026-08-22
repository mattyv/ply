//! Shared helpers for the syntax test suites.

use ply_diag::{Diagnostics, SourceMap};
use ply_syntax::ast::File;
use ply_syntax::dump;

pub fn parse(src: &str) -> (File, Diagnostics, SourceMap) {
    let mut sm = SourceMap::new();
    let file = sm.add("test.ply", src);
    let mut diags = Diagnostics::new();
    let ast = ply_syntax::parse_file(file, &sm.source(file), &mut diags);
    (ast, diags, sm)
}

/// Parse and assert there were no diagnostics.
pub fn parse_ok(src: &str) -> File {
    let (ast, diags, sm) = parse(src);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics for {src:?}:\n{}",
        ply_diag::render_all(&sm, &diags, ply_diag::Color::Never)
    );
    ast
}

/// Compact s-expression dump of a successfully parsed file.
pub fn dump_src(src: &str) -> String {
    dump::dump_file(&parse_ok(src))
}

/// The diagnostic codes produced, in source order.
pub fn errors(src: &str) -> Vec<String> {
    let (_, diags, _) = parse(src);
    diags.sorted().iter().map(|d| d.code.as_str().to_string()).collect()
}

/// Render the diagnostics as text (for eyeballing a failure).
#[allow(dead_code)]
pub fn rendered(src: &str) -> String {
    let (_, diags, sm) = parse(src);
    ply_diag::render_all(&sm, &diags.sorted(), ply_diag::Color::Never)
}

/// Dump an expression parsed on its own.
#[allow(dead_code)]
pub fn dump_expr(src: &str) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add("expr.ply", src);
    let mut diags = Diagnostics::new();
    let e = ply_syntax::parse_expression(file, &sm.source(file), &mut diags);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics for {src:?}:\n{}",
        ply_diag::render_all(&sm, &diags, ply_diag::Color::Never)
    );
    dump::dump_expr(&e)
}

/// Diagnostic codes from parsing a bare expression.
#[allow(dead_code)]
pub fn expr_errors(src: &str) -> Vec<String> {
    let mut sm = SourceMap::new();
    let file = sm.add("expr.ply", src);
    let mut diags = Diagnostics::new();
    let _ = ply_syntax::parse_expression(file, &sm.source(file), &mut diags);
    diags.sorted().iter().map(|d| d.code.as_str().to_string()).collect()
}
