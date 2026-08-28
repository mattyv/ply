//! Fixture for D5's *first* branch (The-Ply-Spec.md §5.5): `g` is claimed
//! and proved bounded(2) in this same crate, and `f` calls it. Ply must
//! stub `g` with `#[kani::stub_verified]` (standing on `g`'s own proof)
//! instead of assuming its contract, so `f` comes back clean -- not
//! `conditional`, not carrying owed evidence for `g`.

#[ply::requires(x < 1000)]
#[ply::ensures(|result| *result == x + 1)]
pub fn g(x: u32) -> u32 {
    x + 1
}

#[ply::requires(x < 999)]
#[ply::ensures(|result| *result == x + 2)]
pub fn f(x: u32) -> u32 {
    g(g(x))
}
