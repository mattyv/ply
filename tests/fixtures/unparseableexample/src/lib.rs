//! One sound function. The defect this fixture exists for is entirely in
//! `ply.yaml`: one of its `examples` entries is not a Rust expression at
//! all, so it never reaches the compiler that would have objected to it.

#[ply::requires(a < 100 && b < 100)]
#[ply::ensures(|result| *result == a + b)]
pub fn add_small(a: u8, b: u8) -> u8 {
    a + b
}
