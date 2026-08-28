//! D1's reproduction (adversarial review, 2026-08-26): min-composition is
//! unsound when the callee's own proof does not cover the caller's
//! argument. `g` honours its promise only for vectors of length <= 2 (its
//! own `bounded(2)` proof only ever builds vectors up to that length) --
//! for anything longer it returns 99, breaking `*result <= 10`. `f` always
//! passes a vector of length 3, outside every domain `g`'s own proof ever
//! covered. Branch one composing `f`'s bound to `g`'s would report `f`
//! clean at `bounded(2)` while the real `f` violates its own contract on
//! every input -- so `g`'s `Vec<u8>` parameter must exclude it from branch
//! one entirely, falling `f` back to branch two (assumed, `conditional`)
//! instead.

#[ply::ensures(|result| *result <= 10)]
pub fn g(v: Vec<u8>) -> u32 {
    if v.len() > 2 { 99 } else { 0 }
}

#[ply::ensures(|result| *result <= 10)]
pub fn f(x: u32) -> u32 {
    // A single fixed-length literal, not a sequence of `push`es: the point
    // of this fixture is `g`'s Vec<u8> domain-coverage gap, not an
    // unrelated capacity-growth loop inside `f` competing for the same
    // unwind bound.
    let w = vec![x as u8, 0, 0];
    g(w)
}
