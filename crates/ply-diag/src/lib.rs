//! Diagnostics for the Ply compiler: one model, two surfaces (JSON and human text).
//!
//! `ply-diag` is a leaf crate — every other crate depends on it and it depends on none of
//! them (§4).

pub mod code;
pub mod diagnostic;
pub mod render;
pub mod span;

pub use code::{Code, Phase, Severity};
pub use diagnostic::{
    Counterexample, Diagnostic, DiagnosticJson, Diagnostics, Edit, Fix, Related, TraceEntry,
    TraceEvent, apply_fix,
};
pub use render::{Color, render, render_all};
pub use span::{FileId, LineCol, SourceMap, Span};
