/// This block implements `inner::Root` (`super::` from inside `inner::sub`
/// is `inner`) -- a different type again from `crate::Root` in
/// `inner.rs`, carrying no promise of its own at all. The old resolver
/// matched a claim written `inner::Root::five` against `inner.rs`'s
/// `impl super::Root` purely because the bare name "Root" and the
/// recursion frame lined up, and ran THIS function's body while reporting
/// THAT one's promise. This fixture pins that a claim spelled
/// `inner::Root::five` resolves here, to its own real declaration, and
/// never borrows the crate-root one's promise.
impl super::Root {
    pub fn five() -> u32 {
        999
    }
}
