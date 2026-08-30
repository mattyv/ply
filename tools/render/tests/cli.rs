//! Exercises the actual `ply-render` binary (not just the library), since
//! argument parsing, exit codes, and stdout-vs-file output all live in
//! `main.rs`, not in `render_svg_with_options`. Modeled on
//! `tools/check/tests/cli.rs`.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ply-render"))
        .args(args)
        .output()
        .expect("ply-render should run")
}

#[test]
fn renders_svg_to_stdout_when_no_output_path_given() {
    let out = run(&["tests/fixtures/full.ply.yaml"]);
    assert!(out.status.success(), "expected exit 0, got: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("<svg"),
        "stdout should carry the SVG, got: {stdout:?}"
    );
    assert!(out.stderr.is_empty(), "expected no stderr, got: {out:?}");
}

#[test]
fn writes_svg_to_file_when_out_flag_given() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("ply-render-cli-test-{}.svg", std::process::id()));
    let out = run(&["tests/fixtures/full.ply.yaml", "-o", path.to_str().unwrap()]);
    assert!(out.status.success(), "expected exit 0, got: {out:?}");
    assert!(
        out.stdout.is_empty(),
        "with -o, stdout should carry nothing, got: {out:?}"
    );
    let written = std::fs::read_to_string(&path).expect("output file should exist");
    assert!(written.starts_with("<svg"));
    std::fs::remove_file(&path).ok();
}

#[test]
fn missing_input_file_is_an_error() {
    let out = run(&["tests/fixtures/does_not_exist.ply.yaml"]);
    assert!(!out.status.success(), "expected nonzero exit, got: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("error: could not read tests/fixtures/does_not_exist.ply.yaml:"),
        "got: {stderr:?}"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("<svg"),
        "a failed run must never emit SVG on stdout, got: {out:?}"
    );
}

#[test]
fn unparseable_input_file_is_an_error() {
    let out = run(&["tests/fixtures/unparseable.ply.yaml"]);
    assert!(!out.status.success(), "expected nonzero exit, got: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("error: tests/fixtures/unparseable.ply.yaml did not parse as ply.yaml:"),
        "got: {stderr:?}"
    );
    assert!(stderr.contains("not_a_real_field"), "got: {stderr:?}");
}

#[test]
fn depth_flag_collapses_non_hollow_top_level_components() {
    let out = run(&["tests/fixtures/full.ply.yaml", "--depth", "1"]);
    assert!(out.status.success(), "expected exit 0, got: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // full.ply.yaml's top level is parser, pricing, risk, db_raw,
    // migrations; the last three are hollow (no fns, no nested components)
    // and a hollow box never collapses (§7.1: hollow means nothing inside,
    // collapsed means plenty inside folded — mutually exclusive), so
    // --depth 1 only folds parser and pricing.
    assert_eq!(
        stdout.matches("class=\"collapsed-stack ").count(),
        2,
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("data-name=\"curves\""),
        "pricing collapsed, so its nested curves must not draw its own box, got: {stdout}"
    );
}

#[test]
fn focus_flag_expands_only_the_named_subtree() {
    let default = run(&["tests/fixtures/full.ply.yaml"]);
    let default_stdout = String::from_utf8_lossy(&default.stdout);
    assert_eq!(
        default_stdout.matches("class=\"collapsed-stack ").count(),
        0,
        "the no-flags baseline must never collapse anything"
    );

    let out = run(&["tests/fixtures/full.ply.yaml", "--focus", "pricing.curves"]);
    assert!(out.status.success(), "expected exit 0, got: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Only parser is both unrelated to the focus path and non-hollow (risk,
    // db_raw, migrations are also unrelated, but hollow never collapses);
    // pricing (ancestor) and curves (the target) must stay expanded.
    assert_eq!(
        stdout.matches("class=\"collapsed-stack ").count(),
        1,
        "got: {stdout}"
    );
    assert!(
        stdout.contains("data-name=\"curves\""),
        "curves is the focus target, so it must still draw its own box, got: {stdout}"
    );
}

#[test]
fn repeated_collapse_flags_all_apply() {
    let out = run(&[
        "tests/fixtures/full.ply.yaml",
        "--collapse",
        "parser",
        "--collapse",
        "pricing",
    ]);
    assert!(out.status.success(), "expected exit 0, got: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.matches("class=\"collapsed-stack ").count(),
        2,
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("data-name=\"curves\""),
        "pricing collapsed (2nd --collapse flag), so curves must not draw its own box, got: \
         {stdout}"
    );
}

#[test]
fn unknown_focus_component_is_an_error_that_says_what_to_do() {
    let out = run(&["tests/fixtures/full.ply.yaml", "--focus", "nope"]);
    assert!(!out.status.success(), "expected nonzero exit, got: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.trim_end(),
        "error: tests/fixtures/full.ply.yaml could not be rendered: --focus \"nope\" does not \
         match any component in this document — check the spelling against the names in \
         ply.yaml (a nested component uses its dotted path, e.g. parent.child)"
    );
}

#[test]
fn unknown_collapse_component_is_an_error_that_says_what_to_do() {
    let out = run(&["tests/fixtures/full.ply.yaml", "--collapse", "nope"]);
    assert!(!out.status.success(), "expected nonzero exit, got: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.trim_end(),
        "error: tests/fixtures/full.ply.yaml could not be rendered: --collapse \"nope\" does \
         not match any component in this document — check the spelling against the names in \
         ply.yaml (a nested component uses its dotted path, e.g. parent.child)"
    );
}

#[test]
fn depth_zero_is_rejected_with_a_message_that_says_what_to_do() {
    let out = run(&["tests/fixtures/full.ply.yaml", "--depth", "0"]);
    assert!(!out.status.success(), "expected nonzero exit, got: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "--depth 0 doesn't select anything: nesting levels start at 1 for the top-level \
             boxes"
        ),
        "got: {stderr:?}"
    );
    assert!(
        stderr.contains("--depth 1 or higher"),
        "the message must say what to do instead, got: {stderr:?}"
    );
}

#[test]
fn non_numeric_depth_is_rejected_with_a_plain_language_message() {
    let out = run(&["tests/fixtures/full.ply.yaml", "--depth", "abc"]);
    assert!(!out.status.success(), "expected nonzero exit, got: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "--depth wants a whole number of nesting levels, counting the top-level boxes as 1"
        ),
        "got: {stderr:?}"
    );
    assert!(stderr.contains("\"abc\""), "got: {stderr:?}");
}

/// The text form has to be reachable from the command line, or it is a
/// library function nobody can run. It goes to the same places the drawing
/// does — stdout by default, a file with `-o` — so a reader can pipe it
/// into anything without learning a second set of rules.
#[test]
fn text_flag_writes_the_transcript_to_stdout_instead_of_the_drawing() {
    let out = run(&["tests/fixtures/full.ply.yaml", "--text"]);
    assert!(out.status.success(), "expected exit 0, got: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("<svg"),
        "--text asked for the text form and got a drawing, got: {stdout:?}"
    );
    assert!(
        stdout.starts_with("This is a Ply transcript:"),
        "the text form should open by saying what it is, got: {stdout:?}"
    );
    assert!(out.stderr.is_empty(), "expected no stderr, got: {out:?}");
}

#[test]
fn text_flag_honours_the_output_path() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("ply-render-cli-text-{}.txt", std::process::id()));
    let out = run(&[
        "tests/fixtures/full.ply.yaml",
        "--text",
        "-o",
        path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "expected exit 0, got: {out:?}");
    assert!(
        out.stdout.is_empty(),
        "with -o, stdout should carry nothing, got: {out:?}"
    );
    let written = std::fs::read_to_string(&path).expect("output file should exist");
    assert!(
        written.starts_with("This is a Ply transcript:"),
        "got: {written:?}"
    );
    std::fs::remove_file(&path).ok();
}

/// `--depth`, `--focus` and `--collapse` fold parts of the *drawing* away to
/// fit a screen. The text form has no screen to fit and always states the
/// whole document, so combining them is a request that cannot be honoured.
/// Saying so beats silently ignoring the flag — the reader would otherwise
/// believe they were handed a narrowed view.
#[test]
fn text_flag_says_plainly_that_the_folding_flags_do_not_apply_to_it() {
    let out = run(&["tests/fixtures/full.ply.yaml", "--text", "--depth", "1"]);
    assert!(!out.status.success(), "expected nonzero exit, got: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr,
        "error: --text writes out the whole document, so it cannot be combined with --depth, \
         --focus or --collapse. Those fold parts of the drawing away to fit a screen; the text \
         form has no screen to fit. Drop --depth to get the text, or drop --text to get a \
         folded drawing.\n",
        "got: {stderr:?}"
    );
    assert!(
        out.stdout.is_empty(),
        "a refused run must emit nothing on stdout, got: {out:?}"
    );
}
