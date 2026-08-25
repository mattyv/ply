//! Fixture for The-Ply-Spec.md §5.2a: the shape whose absence let a stored
//! result lie.
//!
//! Every claim in `resultreuse` is either a leaf or crosses only into a
//! callee covered by a declared promise -- the one callee shape the
//! fingerprint used to cover. Nothing in it calls an **ordinary local
//! helper**, which is what almost all real code does, and which the
//! generated test runs straight through. So breaking the helper left the
//! claim's own tokens untouched, the fingerprint matched, and a broken
//! function was carried forward as a confident pass.
//!
//! Two independent claims, so that re-running one is visibly not
//! re-running the other.

/// `doubled`'s helper. No contract on it, no promise declared for it in
/// ply.yaml: ordinary code, which the sampling tier simply calls.
pub fn scale(x: u32) -> u32 {
    x * 2
}

/// Breaks its own guarantee the moment `scale` does.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result >= x)]
pub fn doubled(x: u32) -> u32 {
    scale(x)
}

/// `bumped`'s helper, reachable from nothing else.
pub fn shift(x: u32) -> u32 {
    x + 1
}

/// The control: editing `scale` must leave this claim carried forward, or
/// the fix has bought soundness by throwing per-claim reuse away.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result > x)]
pub fn bumped(x: u32) -> u32 {
    shift(x)
}
