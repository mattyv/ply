use crate::pairs::Pair;

/// The sibling of `pair_ops_a.rs`'s `impl Pair<u8>` -- a different
/// concrete instantiation, in a different file.
impl Pair<u16> {
    pub fn describe(&self) -> u32 {
        2
    }
}
