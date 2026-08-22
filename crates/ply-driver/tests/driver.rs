//! The front-end driver: discovery, `check`, `fmt` and the two output surfaces (§14).

use ply_driver::{Analysis, CheckOutcome, FmtOutcome};

fn analyze(src: &str) -> Analysis {
    Analysis::of_sources(vec![("test.ply".to_string(), src.to_string())])
}

#[test]
fn a_clean_program_reports_nothing() {
    let a = analyze("fn main() -> () {}\n");
    assert!(!a.has_errors());
    assert_eq!(a.outcome(), CheckOutcome::Clean);
    assert_eq!(a.human(ply_diag::Color::Never), "");
    assert_eq!(a.json(), "[]");
}

#[test]
fn check_reports_parse_and_naming_problems_together() {
    let a = analyze("fn badName() -> Int {\n    let x = 1\n    x\n}\n");
    let codes: Vec<&str> = a.diagnostics().iter().map(|d| d.code.as_str()).collect();
    assert_eq!(codes, vec!["E0101", "E0110"]);
    assert_eq!(a.outcome(), CheckOutcome::Errors(2));
}

#[test]
fn diagnostics_are_sorted_by_position_across_files() {
    let a = Analysis::of_sources(vec![
        ("b.ply".into(), "fn Two() -> () {}\n".into()),
        ("a.ply".into(), "fn One() -> () {}\n".into()),
    ]);
    let files: Vec<String> = a
        .diagnostics()
        .iter()
        .map(|d| a.sources().name(d.primary_span.file).to_string())
        .collect();
    assert_eq!(files, vec!["b.ply", "a.ply"], "files keep their command-line order");
}

#[test]
fn json_output_matches_the_documented_shape() {
    let a = analyze("fn f() -> Int {\n    let x = 1\n    x\n}\n");
    let v: serde_json::Value = serde_json::from_str(&a.json()).unwrap();
    let d = &v[0];
    assert_eq!(d["code"], "E0110");
    assert_eq!(d["severity"], "error");
    assert_eq!(d["phase"], "parse");
    assert_eq!(d["primary_span"]["file"], "test.ply");
    assert_eq!(d["primary_span"]["start"][0], 2);
    assert!(d["worklist_ref"].is_null());
    assert_eq!(d["fixes"][0]["title"], "add `;`");
    assert_eq!(d["fixes"][0]["edits"][0]["insert"], ";");
}

#[test]
fn human_output_shows_the_source_line_and_a_summary() {
    let a = analyze("fn f() -> Int {\n    let x = 1\n    x\n}\n");
    let text = a.human(ply_diag::Color::Never);
    assert!(text.contains("error[E0110]"), "{text}");
    assert!(text.contains("--> test.ply:2:14"), "{text}");
    assert!(text.contains("1 error"), "{text}");
}

#[test]
fn fmt_reports_whether_a_file_would_change() {
    assert_eq!(
        ply_driver::format_source("a.ply", "fn f()->(){}"),
        FmtOutcome::Changed("fn f() -> () {}\n".to_string())
    );
    assert_eq!(
        ply_driver::format_source("a.ply", "fn f() -> () {}\n"),
        FmtOutcome::Unchanged
    );
}

#[test]
fn fmt_refuses_to_touch_a_file_that_does_not_parse() {
    match ply_driver::format_source("a.ply", "fn f( {") {
        FmtOutcome::Failed(d) => assert!(d.has_errors()),
        other => panic!("expected a parse failure, got {other:?}"),
    }
}

#[test]
fn discovery_finds_ply_files_and_ignores_the_rest() {
    let dir = tempdir();
    std::fs::write(dir.join("a.ply"), "fn a() -> () {}\n").unwrap();
    std::fs::write(dir.join("notes.md"), "hello").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/b.ply"), "fn b() -> () {}\n").unwrap();
    std::fs::create_dir(dir.join("target")).unwrap();
    std::fs::write(dir.join("target/c.ply"), "fn c() -> () {}\n").unwrap();

    let found = ply_driver::collect_ply_files(&dir).unwrap();
    let names: Vec<String> = found
        .iter()
        .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(names, vec!["a.ply", "sub/b.ply"], "target/ is skipped");

    // A single file resolves to itself.
    let one = ply_driver::collect_ply_files(&dir.join("a.ply")).unwrap();
    assert_eq!(one, vec![dir.join("a.ply")]);
}

#[test]
fn discovery_reports_a_missing_path() {
    assert!(ply_driver::collect_ply_files(std::path::Path::new("/nope/nowhere")).is_err());
}

fn tempdir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "ply-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}
