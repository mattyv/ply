#[ply::ensures(|result| *result <= 10)]
pub fn g((a, b): (u32, u32)) -> u32 {
    if a > b {
        0
    } else {
        1
    }
}

#[ply::ensures(|result| *result <= 10)]
pub fn f(x: u32) -> u32 {
    g((x, x))
}
