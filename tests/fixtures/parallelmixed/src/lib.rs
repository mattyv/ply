#[ply::ensures(|result| *result == x)]
pub fn bounded_claim(x: u32) -> u32 {
    x
}

#[ply::ensures(|result| *result == x)]
pub fn sampled_claim(x: u32) -> u32 {
    x
}
