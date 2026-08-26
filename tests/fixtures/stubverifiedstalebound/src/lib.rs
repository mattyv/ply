//! Fixture for §5.2a's fingerprint composing against D5's first branch
//! (The-Ply-Spec.md §5.5): `f` stands on `g`'s own earned `bounded(k)`, so
//! `f`'s *record* must depend on that `k`, not only on `g`'s source. `g`'s
//! source never changes in this fixture at all -- only its declared
//! `checks:` bound does, between two runs, in the test that uses this.

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
