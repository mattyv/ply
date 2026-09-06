//! A path dependency declared under a platform predicate rather than under
//! plain `[dependencies]`. Its source is compiled and run exactly the same
//! way; the only difference is the table header, which is what Ply used to
//! read wrongly.

/// The body the fixture's e2e test rewrites.
pub fn scale(x: u32) -> u32 {
    x * 2
}
