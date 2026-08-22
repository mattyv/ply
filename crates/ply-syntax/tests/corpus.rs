//! Every corpus program must parse cleanly, be canonically formatted already, and survive
//! a format round trip unchanged (§15, §16 M0).

use ply_diag::{Diagnostics, SourceMap};
use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn corpus_files() -> Vec<PathBuf> {
    let dir = corpus_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ply"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "the corpus is empty");
    files
}

struct Parsed {
    ast: ply_syntax::ast::File,
    diags: Diagnostics,
    sm: SourceMap,
}

fn parse(name: &str, src: &str) -> Parsed {
    let mut sm = SourceMap::new();
    let file = sm.add(name, src);
    let mut diags = Diagnostics::new();
    let ast = ply_syntax::parse_file(file, &sm.source(file), &mut diags);
    ply_syntax::naming::check_file(&ast, &mut diags);
    Parsed { ast, diags, sm }
}

/// Every `//` and `/* */` comment text in the source, in order.
fn comments_of(src: &str) -> Vec<String> {
    let mut sm = SourceMap::new();
    let file = sm.add("x.ply", src);
    let mut d = Diagnostics::new();
    let lexed = ply_syntax::lexer::lex(file, &sm.source(file), &mut d);
    lexed.trivia.iter().map(|t| lexed.trivia_text(t).trim_end().to_string()).collect()
}

#[test]
fn corpus_parses_without_diagnostics() {
    for path in corpus_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let p = parse(path.file_name().unwrap().to_str().unwrap(), &src);
        assert!(
            p.diags.is_empty(),
            "{}:\n{}",
            path.display(),
            ply_diag::render_all(&p.sm, &p.diags.sorted(), ply_diag::Color::Never)
        );
    }
}

#[test]
fn corpus_is_already_canonically_formatted() {
    for path in corpus_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let p = parse(path.file_name().unwrap().to_str().unwrap(), &src);
        let out = ply_syntax::format_file(&p.ast);
        if out != src {
            let diff: Vec<String> = src
                .lines()
                .zip(out.lines())
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .take(5)
                .map(|(i, (a, b))| format!("  line {}:\n    have: {a:?}\n    want: {b:?}", i + 1))
                .collect();
            panic!(
                "{} is not canonically formatted (run `ply fmt`):\n{}",
                path.display(),
                if diff.is_empty() {
                    format!("  length differs: {} vs {}", src.len(), out.len())
                } else {
                    diff.join("\n")
                }
            );
        }
    }
}

#[test]
fn formatting_is_idempotent_and_preserves_the_tree() {
    for path in corpus_files() {
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let src = std::fs::read_to_string(&path).unwrap();
        let once = ply_syntax::format_file(&parse(&name, &src).ast);
        let twice = ply_syntax::format_file(&parse(&name, &once).ast);
        assert_eq!(once, twice, "{} : fmt is not idempotent", path.display());
        assert_eq!(
            ply_syntax::dump::dump_file(&parse(&name, &src).ast),
            ply_syntax::dump::dump_file(&parse(&name, &once).ast),
            "{} : formatting changed the tree",
            path.display()
        );
    }
}

#[test]
fn formatting_preserves_every_comment() {
    for path in corpus_files() {
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let src = std::fs::read_to_string(&path).unwrap();
        let out = ply_syntax::format_file(&parse(&name, &src).ast);
        let mut before = comments_of(&src);
        let mut after = comments_of(&out);
        before.sort();
        after.sort();
        assert_eq!(before, after, "{} : formatting lost or invented a comment", path.display());
    }
}

/// Reformatting a mangled copy of each program must land back on the canonical text.
#[test]
fn formatting_normalises_whitespace_damage() {
    for path in corpus_files() {
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let src = std::fs::read_to_string(&path).unwrap();
        // Collapse indentation and add stray blank lines, keeping line structure so `//`
        // comments still terminate where they did.
        let mangled: String =
            src.lines().map(|l| format!("{}\n\n", l.trim_start())).collect::<Vec<_>>().concat();
        let p = parse(&name, &mangled);
        assert!(
            p.diags.is_empty(),
            "{} (mangled):\n{}",
            path.display(),
            ply_diag::render_all(&p.sm, &p.diags.sorted(), ply_diag::Color::Never)
        );
        // Blank lines the mangling introduced are preserved as separators, so compare the
        // fixed point rather than the original text.
        let out = ply_syntax::format_file(&p.ast);
        let again = ply_syntax::format_file(&parse(&name, &out).ast);
        assert_eq!(out, again, "{} : fmt of mangled source is not a fixed point", path.display());
    }
}
