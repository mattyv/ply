//! Fixture for The-Ply-Spec.md §5.2a's coarse mode: a crate whose source
//! contains an `impl` block, which is the commonest reason Ply abandons the
//! call walk.
//!
//! A syntactic walk can follow `helper(x)`. It cannot follow `s.apply(x)`:
//! which body that names depends on the receiver's type, and resolving that
//! needs a type checker. So Ply stops guessing and hashes every line of the
//! crate instead -- correct, and blunt enough that a person needs telling
//! why their unrelated claim was checked again.

/// The type whose mere existence widens the scope.
pub struct Scaler;

impl Scaler {
    pub fn apply(&self, x: u32) -> u32 {
        x / 2
    }
}

/// Reaches the method, which the walk cannot follow.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result <= x)]
pub fn halved(x: u32) -> u32 {
    Scaler.apply(x)
}

/// Touches nothing else in the crate. Under the coarse mode it is still
/// re-run when `halved` moves, and that is the price being explained.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result >= x)]
pub fn bumped(x: u32) -> u32 {
    x + 1
}
