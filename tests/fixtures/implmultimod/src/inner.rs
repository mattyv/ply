pub mod sub;

/// A DIFFERENT `Root` from the crate-root one in `lib.rs` -- same bare
/// name, unrelated type, declared here in `inner`.
pub struct Root;

/// This block implements `crate::Root` (`super::` from inside `inner` is
/// the crate root), NOT `inner::Root` right above it -- and carries the
/// one real promise in this crate: the answer is 999. Its own body says
/// otherwise, on purpose: a run that reports this promise as holding has
/// attached it to the wrong function.
impl super::Root {
    #[ply::ensures(|result| *result == 999)]
    pub fn five() -> u32 {
        5
    }
}
