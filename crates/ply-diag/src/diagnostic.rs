//! The `Diagnostic` model (§13).
//!
//! Every message — parse error, type error, borrow conflict, verifier counterexample,
//! runtime trap — is one of these. The JSON projection ([`DiagnosticJson`]) is a stable API:
//! it is what an agent consumes, so it is built from an explicit mirror type rather than
//! from `#[derive(Serialize)]` on the internal representation.

use crate::code::{Code, Phase, Severity};
use crate::span::{Span, SourceMap};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: Code,
    pub severity: Severity,
    pub phase: Phase,
    pub title: String,
    pub primary_span: Span,
    pub related: Vec<Related>,
    pub counterexample: Option<Counterexample>,
    pub fixes: Vec<Fix>,
    pub worklist_ref: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Related {
    pub span: Span,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counterexample {
    /// Rendered input values, in parameter order.
    pub inputs: Vec<(String, String)>,
    /// e.g. `"bounded(depth: 2)"`.
    pub verdict_context: String,
    pub trace: Vec<TraceEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub span: Span,
    pub event: TraceEvent,
    pub detail: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TraceEvent {
    Assign,
    Call,
    Ret,
    Branch,
    Trap,
    Contract,
}

/// A concrete, mechanically applicable edit. Never vague prose (§13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub title: String,
    pub edits: Vec<Edit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// A zero-width span means "insert here"; otherwise the range is replaced.
    pub span: Span,
    pub text: String,
}

impl Edit {
    pub fn insert(span: Span, text: impl Into<String>) -> Edit {
        Edit { span: span.at_start(), text: text.into() }
    }

    pub fn insert_after(span: Span, text: impl Into<String>) -> Edit {
        Edit { span: span.at_end(), text: text.into() }
    }

    pub fn replace(span: Span, text: impl Into<String>) -> Edit {
        Edit { span, text: text.into() }
    }

    pub fn delete(span: Span) -> Edit {
        Edit { span, text: String::new() }
    }

    pub fn is_insertion(&self) -> bool {
        self.span.is_empty()
    }
}

impl Fix {
    pub fn new(title: impl Into<String>, edits: Vec<Edit>) -> Fix {
        Fix { title: title.into(), edits }
    }

    /// A one-edit fix that inserts `text` at the start of `span`.
    pub fn insert(title: impl Into<String>, span: Span, text: impl Into<String>) -> Fix {
        Fix::new(title, vec![Edit::insert(span, text)])
    }

    /// A one-edit fix that inserts `text` immediately after `span`.
    pub fn insert_after(title: impl Into<String>, span: Span, text: impl Into<String>) -> Fix {
        Fix::new(title, vec![Edit::insert_after(span, text)])
    }

    pub fn replace(title: impl Into<String>, span: Span, text: impl Into<String>) -> Fix {
        Fix::new(title, vec![Edit::replace(span, text)])
    }
}

impl Diagnostic {
    pub fn new(code: Code, primary_span: Span, title: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            severity: code.severity(),
            phase: code.phase(),
            title: title.into(),
            primary_span,
            related: Vec::new(),
            counterexample: None,
            fixes: Vec::new(),
            worklist_ref: None,
        }
    }

    pub fn with_related(mut self, span: Span, note: impl Into<String>) -> Diagnostic {
        self.related.push(Related { span, note: note.into() });
        self
    }

    pub fn with_fix(mut self, fix: Fix) -> Diagnostic {
        self.fixes.push(fix);
        self
    }

    pub fn with_counterexample(mut self, cx: Counterexample) -> Diagnostic {
        self.counterexample = Some(cx);
        self
    }

    pub fn with_worklist_ref(mut self, id: u32) -> Diagnostic {
        self.worklist_ref = Some(id);
        self
    }

    /// Override the registry default (e.g. demoting an error under `--dev`).
    pub fn with_severity(mut self, sev: Severity) -> Diagnostic {
        self.severity = sev;
        self
    }

    pub fn is_error(&self) -> bool {
        matches!(self.severity, Severity::Error | Severity::Ice)
    }

    pub fn to_json(&self, sm: &SourceMap) -> DiagnosticJson {
        DiagnosticJson {
            code: self.code.as_str(),
            severity: self.severity,
            phase: self.phase,
            title: self.title.clone(),
            primary_span: SpanJson::new(sm, self.primary_span),
            related: self
                .related
                .iter()
                .map(|r| RelatedJson { span: SpanJson::new(sm, r.span), note: r.note.clone() })
                .collect(),
            counterexample: self.counterexample.as_ref().map(|c| CounterexampleJson {
                inputs: c.inputs.iter().cloned().collect(),
                verdict_context: c.verdict_context.clone(),
                trace: c
                    .trace
                    .iter()
                    .map(|t| TraceJson {
                        span: SpanJson::new(sm, t.span),
                        event: t.event,
                        detail: t.detail.clone(),
                    })
                    .collect(),
            }),
            fixes: self
                .fixes
                .iter()
                .map(|f| FixJson {
                    title: f.title.clone(),
                    edits: f.edits.iter().map(|e| EditJson::new(sm, e)).collect(),
                })
                .collect(),
            worklist_ref: self.worklist_ref,
        }
    }
}

// ---------------------------------------------------------------------------------------
// JSON projection
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticJson {
    pub code: &'static str,
    pub severity: Severity,
    pub phase: Phase,
    pub title: String,
    pub primary_span: SpanJson,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<CounterexampleJson>,
    #[serde(default)]
    pub fixes: Vec<FixJson>,
    pub worklist_ref: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpanJson {
    pub file: String,
    /// `[line, column]`, both 1-based.
    pub start: [u32; 2],
    pub end: [u32; 2],
}

impl SpanJson {
    fn new(sm: &SourceMap, span: Span) -> SpanJson {
        let s = sm.start(span);
        let e = sm.end(span);
        SpanJson {
            file: sm.name(span.file).to_string(),
            start: [s.line, s.col],
            end: [e.line, e.col],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelatedJson {
    pub span: SpanJson,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CounterexampleJson {
    pub inputs: Vec<(String, String)>,
    pub verdict_context: String,
    pub trace: Vec<TraceJson>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceJson {
    pub span: SpanJson,
    pub event: TraceEvent,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixJson {
    pub title: String,
    pub edits: Vec<EditJson>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditJson {
    pub span: SpanJson,
    /// Present when the edit is a pure insertion (zero-width span).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert: Option<String>,
    /// Present when the edit replaces the spanned text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
}

impl EditJson {
    fn new(sm: &SourceMap, e: &Edit) -> EditJson {
        let span = SpanJson::new(sm, e.span);
        if e.is_insertion() {
            EditJson { span, insert: Some(e.text.clone()), replace: None }
        } else {
            EditJson { span, insert: None, replace: Some(e.text.clone()) }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------------------

/// An ordered bag of diagnostics with the "one root cause" bookkeeping (§13).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Diagnostics {
        Diagnostics::default()
    }

    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }

    pub fn extend(&mut self, other: impl IntoIterator<Item = Diagnostic>) {
        self.items.extend(other);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.items.iter()
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    pub fn error_count(&self) -> usize {
        self.items.iter().filter(|d| d.is_error()).count()
    }

    pub fn warning_count(&self) -> usize {
        self.items.iter().filter(|d| d.severity == Severity::Warning).count()
    }

    /// Stable source order: by file, then start offset, then code.
    pub fn sorted(mut self) -> Diagnostics {
        self.items.sort_by_key(|d| {
            (d.primary_span.file, d.primary_span.start, d.primary_span.end, d.code.as_str())
        });
        self
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }

    pub fn to_json(&self, sm: &SourceMap) -> Vec<DiagnosticJson> {
        self.items.iter().map(|d| d.to_json(sm)).collect()
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl FromIterator<Diagnostic> for Diagnostics {
    fn from_iter<T: IntoIterator<Item = Diagnostic>>(iter: T) -> Diagnostics {
        Diagnostics { items: iter.into_iter().collect() }
    }
}

/// Apply a fix to source text. Edits must not overlap; they are applied back-to-front.
pub fn apply_fix(source: &str, fix: &Fix) -> String {
    let mut edits: Vec<&Edit> = fix.edits.iter().collect();
    edits.sort_by_key(|e| std::cmp::Reverse((e.span.start, e.span.end)));
    let mut out = source.to_string();
    for e in edits {
        let start = (e.span.start as usize).min(out.len());
        let end = (e.span.end as usize).min(out.len()).max(start);
        out.replace_range(start..end, &e.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_fixes_render_as_insert() {
        let mut sm = SourceMap::new();
        let f = sm.add("a.ply", "let x = 1\n");
        let span = Span::new(f, 8, 9);
        let d = Diagnostic::new(Code::E0110, span, "expected `;`")
            .with_fix(Fix::insert_after("add `;`", span, ";"));
        let j = d.to_json(&sm);
        assert_eq!(j.code, "E0110");
        assert_eq!(j.fixes[0].edits[0].insert.as_deref(), Some(";"));
        assert_eq!(j.fixes[0].edits[0].span.start, [1, 10]);
    }

    #[test]
    fn apply_fix_inserts_text() {
        let mut sm = SourceMap::new();
        let f = sm.add("a.ply", "let x = 1\n");
        let fix = Fix::insert_after("add `;`", Span::new(f, 8, 9), ";");
        assert_eq!(apply_fix("let x = 1\n", &fix), "let x = 1;\n");
    }

    #[test]
    fn multi_edit_fixes_apply_back_to_front() {
        let mut sm = SourceMap::new();
        let f = sm.add("a.ply", "abcdef");
        let fix = Fix::new(
            "two edits",
            vec![Edit::replace(Span::new(f, 0, 1), "X"), Edit::replace(Span::new(f, 4, 6), "YY")],
        );
        assert_eq!(apply_fix("abcdef", &fix), "XbcdYY");
    }
}
