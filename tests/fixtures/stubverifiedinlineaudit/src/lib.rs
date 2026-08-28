#[ply::requires(x < 1000)]
#[ply::ensures(|result| *result == x + 1)]
pub fn g(x: u32) -> u32 {
    x + 1
}

#[ply::ensures(|result| *result <= 10)]
pub fn f(x: u32) -> u32 {
    g(x)
}
