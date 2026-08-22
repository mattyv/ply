//! Human-readable rendering of diagnostics: source excerpts, counterexample traces and
//! concrete fixes. Deliberately hand-written so the span model stays unified (§13).

use crate::code::Severity;
use crate::diagnostic::{Diagnostic, Diagnostics, apply_fix};
use crate::span::{Span, SourceMap};
use std::fmt::Write as _;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Color {
    Never,
    Always,
}

impl Color {
    pub fn from_env(is_tty: bool) -> Color {
        if is_tty && std::env::var_os("NO_COLOR").is_none() { Color::Always } else { Color::Never }
    }
}

struct Style {
    color: Color,
}

impl Style {
    fn paint(&self, code: &str, s: &str, out: &mut String) {
        match self.color {
            Color::Never => out.push_str(s),
            Color::Always => {
                out.push_str("\x1b[");
                out.push_str(code);
                out.push('m');
                out.push_str(s);
                out.push_str("\x1b[0m");
            }
        }
    }
}

const RED: &str = "1;31";
const YELLOW: &str = "1;33";
const MAGENTA: &str = "1;35";
const BLUE: &str = "1;34";
const CYAN: &str = "1;36";
const DIM: &str = "2";

fn severity_color(sev: Severity) -> &'static str {
    match sev {
        Severity::Error => RED,
        Severity::Warning => YELLOW,
        Severity::Ice => MAGENTA,
    }
}

/// Render one diagnostic. Output always ends with a newline.
pub fn render(sm: &SourceMap, d: &Diagnostic, color: Color) -> String {
    let st = Style { color };
    let mut out = String::new();

    // error[E0110]: expected `;`
    let head = format!("{}[{}]", d.severity, d.code);
    st.paint(severity_color(d.severity), &head, &mut out);
    out.push_str(": ");
    out.push_str(&d.title);
    out.push('\n');

    let width = gutter_width(sm, d);
    excerpt(sm, &st, d.primary_span, '^', severity_color(d.severity), None, width, &mut out);

    for r in &d.related {
        excerpt(sm, &st, r.span, '-', BLUE, Some(&r.note), width, &mut out);
    }

    if let Some(cx) = &d.counterexample {
        let bar = format!("{:width$} = ", "", width = width);
        st.paint(DIM, &bar, &mut out);
        st.paint(CYAN, "counterexample", &mut out);
        let _ = writeln!(out, " ({})", cx.verdict_context);
        for (name, value) in &cx.inputs {
            let _ = writeln!(out, "{:width$}     {name} = {value}", "", width = width);
        }
        if !cx.trace.is_empty() {
            let _ = writeln!(out, "{:width$}     trace:", "", width = width);
            for t in &cx.trace {
                let lc = sm.start(t.span);
                let _ = writeln!(
                    out,
                    "{:width$}       {}:{:<4} {:<9} {}",
                    "",
                    sm.name(t.span.file),
                    lc.line,
                    format!("{:?}", t.event).to_lowercase(),
                    t.detail,
                    width = width
                );
            }
        }
    }

    for f in &d.fixes {
        st.paint(CYAN, "help", &mut out);
        out.push_str(": ");
        out.push_str(&f.title);
        out.push('\n');
        if let Some(preview) = fix_preview(sm, d.primary_span, f, width) {
            out.push_str(&preview);
        }
    }

    out
}

pub fn render_all(sm: &SourceMap, ds: &Diagnostics, color: Color) -> String {
    let mut out = String::new();
    for d in ds.iter() {
        out.push_str(&render(sm, d, color));
        out.push('\n');
    }
    let (e, w) = (ds.error_count(), ds.warning_count());
    if e > 0 || w > 0 {
        let st = Style { color };
        let summary = match (e, w) {
            (0, w) => format!("{w} warning{}", plural(w)),
            (e, 0) => format!("{e} error{}", plural(e)),
            (e, w) => format!("{e} error{}, {w} warning{}", plural(e), plural(w)),
        };
        st.paint(if e > 0 { RED } else { YELLOW }, &summary, &mut out);
        out.push('\n');
    }
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn gutter_width(sm: &SourceMap, d: &Diagnostic) -> usize {
    let mut max_line = sm.start(d.primary_span).line;
    for r in &d.related {
        max_line = max_line.max(sm.start(r.span).line);
    }
    max_line.to_string().len() + 1
}

/// One `--> file:line:col` header plus the underlined source line.
fn excerpt(
    sm: &SourceMap,
    st: &Style,
    span: Span,
    caret: char,
    caret_color: &'static str,
    note: Option<&str>,
    width: usize,
    out: &mut String,
) {
    let start = sm.start(span);
    let end = sm.end(span);
    let arrow = format!("{:width$}--> ", "", width = width.saturating_sub(1));
    st.paint(DIM, &arrow, out);
    let _ = writeln!(out, "{}:{}:{}", sm.name(span.file), start.line, start.col);

    let empty_gutter = format!("{:width$}| ", "", width = width);
    st.paint(DIM, empty_gutter.trim_end(), out);
    out.push('\n');

    let line_text = sm.line_text(span.file, start.line);
    let num = format!("{:>w$}| ", start.line, w = width);
    st.paint(DIM, &num, out);
    out.push_str(&expand_tabs(line_text));
    out.push('\n');

    // Underline: from the start column to the end column, clamped to this line.
    let last_col = if end.line == start.line {
        end.col.max(start.col + 1)
    } else {
        expand_tabs(line_text).chars().count() as u32 + 1
    };
    let pad = (start.col - 1) as usize;
    let len = (last_col - start.col).max(1) as usize;
    st.paint(DIM, &empty_gutter, out);
    out.push_str(&" ".repeat(pad));
    let mut bar = caret.to_string().repeat(len);
    if end.line > start.line {
        bar.push_str("...");
    }
    st.paint(caret_color, &bar, out);
    if let Some(note) = note {
        out.push(' ');
        st.paint(caret_color, note, out);
    }
    out.push('\n');
}

/// Show the line(s) a fix produces, so the suggestion is literally readable.
fn fix_preview(sm: &SourceMap, primary: Span, fix: &crate::diagnostic::Fix, width: usize) -> Option<String> {
    let file = fix.edits.first()?.span.file;
    if file != primary.file || fix.edits.iter().any(|e| e.span.file != file) {
        return None;
    }
    let source = sm.text(file);
    let fixed = apply_fix(source, fix);
    let first = fix.edits.iter().map(|e| e.span.start).min()?;
    let line = sm.line_col(file, first).line;
    // Recompute line boundaries on the fixed text: an edit may add lines.
    let fixed_line = nth_line(&fixed, line)?;
    let mut out = String::new();
    let _ = writeln!(out, "{:>w$}| {}", line, expand_tabs(fixed_line), w = width);
    Some(out)
}

fn nth_line(text: &str, line: u32) -> Option<&str> {
    text.split('\n').nth(line.saturating_sub(1) as usize)
}

fn expand_tabs(s: &str) -> String {
    s.replace('\t', "    ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::Code;
    use crate::diagnostic::Fix;
    use crate::span::Span;

    #[test]
    fn renders_caret_under_the_span() {
        let mut sm = SourceMap::new();
        let f = sm.add("a.ply", "fn main() -> () {\n    let x = 1\n}\n");
        let span = Span::new(f, 30, 31); // the `1`
        let d = Diagnostic::new(Code::E0110, span, "expected `;`")
            .with_fix(Fix::insert_after("add `;`", span, ";"));
        let text = render(&sm, &d, Color::Never);
        assert!(text.contains("error[E0110]: expected `;`"), "{text}");
        assert!(text.contains("--> a.ply:2:13"), "{text}");
        assert!(text.contains("    let x = 1"), "{text}");
        assert!(text.contains("^"), "{text}");
        assert!(text.contains("help: add `;`"), "{text}");
        assert!(text.contains("let x = 1;"), "{text}");
    }
}
