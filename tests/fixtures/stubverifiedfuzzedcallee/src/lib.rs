//! Fixture proving D5's second branch is unchanged (The-Ply-Spec.md §5.5):
//! `g` carries its own inline contract but is claimed with `fuzz`, never
//! `bounded` -- proptest *runs* it, it never earns a `bounded(k)` verdict --
//! so `f`'s call to it can never qualify for D5's first branch. `f` must
//! still come back `conditional`, exactly as it always did, standing on
//! `g`'s contract rather than a proof of it.

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
