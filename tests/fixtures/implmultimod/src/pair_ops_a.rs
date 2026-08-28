use crate::pairs::Pair;

/// One of two `impl` blocks for `Pair`, each for a different concrete
/// instantiation -- legal Rust (`implambiguous` already pins the same
/// shape in one file). Here the two blocks live in DIFFERENT files, which
/// this crate's canonical-path work now newly makes refusable: ambiguity
/// is not scoped to one file, and Ply must refuse rather than silently
/// resolve to whichever file its crate-wide walk reaches first.
impl Pair<u8> {
    pub fn describe(&self) -> u32 {
        1
    }
}
