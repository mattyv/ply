//! Fixture for D5's first branch, bound composition (The-Ply-Spec.md §5.5):
//! `g` is claimed at `bounded(2)`, `f` calls it but is itself claimed at a
//! *higher* `bounded(5)`. Ply must never report `f` at its own declared
//! bound when it stands on a callee proved only to a shallower one -- the
//! honest composed verdict is `bounded(2)`, the weaker of the two, or Ply
//! would be claiming a depth nothing actually checked.

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
