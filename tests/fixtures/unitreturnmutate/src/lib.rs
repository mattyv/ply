//! A unit-returning helper whose effect is outside the claimed result.
//! Deleting the helper therefore survives the deliberately narrow promise,
//! and `mutate` must report cargo-mutants' `replace reset with ()` wording.

/// A real body prevents cargo-mutants from discarding this as already empty.
/// Its effect is deliberately irrelevant to `capped`'s result contract.
fn reset() {
    std::hint::black_box(());
}

#[ply::ensures(|result| *result <= 10)]
pub fn capped(x: u8) -> u8 {
    reset();
    x.min(10)
}
