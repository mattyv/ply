//! Multi-module method resolution (adversarial review, 2026-08-27, "ninth
//! false clean"). Every shape in this crate needs more than one module to
//! exist at all -- the one arrangement the suite that shipped that defect
//! never exercised, because every earlier method-resolution fixture
//! (`implmethod`, `implambiguous`) is a single file with everything public,
//! the one shape where the path Ply writes is guaranteed to be the path it
//! read.

pub mod inner;
pub mod pair_ops_a;
pub mod pair_ops_b;
pub mod pairs;
pub mod widget_ops;
pub mod widgets;

pub use widgets::Widget as ExportedWidget;

/// A type at the crate root, deliberately sharing its bare name with a
/// DIFFERENT type declared inside `inner` (see `inner::Root`). Its own
/// `impl` block -- and the one real promise in this whole crate -- is
/// written from inside `inner`: ordinary, legal Rust (`impl super::Root`)
/// that the resolver this fixes could not tell apart from `inner`'s own,
/// unrelated `Root`.
pub struct Root;
