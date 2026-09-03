/// Re-exported so existing call sites (`ply_render::model::...`) and tests
/// keep working unchanged. The model itself lives in the product
/// (`ply_core::model`) since Phase 1a -- this renderer consumes it rather
/// than owning a second copy of the `ply.yaml` grammar.
pub use ply_core::model;
pub mod layout;
pub mod svg;

/// The text form of the same facts the drawing shows -- see
/// `ply_core::visual::transcript` for why it exists.
pub use ply_core::visual::transcript;

/// Reading a component's declared state type out of real source -- the
/// half of `state:` the renderer deliberately does not do itself. Same
/// re-export reasoning as `model`: it lives in the product, and this
/// renderer consumes it rather than keeping a second copy.
pub use ply_core::harness;
