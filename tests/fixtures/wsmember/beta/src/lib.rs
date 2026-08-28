//! An ordinary crate-mate with no Ply involvement at all -- present only so
//! `wsmember` is a genuine multi-crate workspace, and so the acceptance
//! test has something innocent to check still builds after `cargo ply
//! verify` runs against `alpha` (docs/review-caveats.md N1).

pub fn double(x: u32) -> u32 {
    x * 2
}
