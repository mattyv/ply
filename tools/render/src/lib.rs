/// Re-exported so existing call sites (`ply_render::model::...`) and tests
/// keep working unchanged. The model itself lives in the product
/// (`ply_core::model`) since Phase 1a -- this renderer consumes it rather
/// than owning a second copy of the `ply.yaml` grammar.
pub use ply_core::model;
pub mod layout;
pub mod svg;
