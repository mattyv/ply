/// Re-exported so existing call sites (`ply_render::model::...`) and tests
/// keep working unchanged now that the model lives in its own crate, shared
/// with `ply-check`.
pub use ply_model as model;
mod layout;
pub mod svg;
