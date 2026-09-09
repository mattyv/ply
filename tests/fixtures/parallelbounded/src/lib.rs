//! Four independent bounded claims: two hold and two have distinct witnesses.

#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn good(x: u32) -> u32 {
    x + 1
}

#[ply::requires(x < u32::MAX - 1)]
#[ply::ensures(|result| *result == x + 2)]
pub fn good_two(x: u32) -> u32 {
    x + 2
}

#[ply::requires(x > 0)]
#[ply::ensures(|result| *result == x + 1)]
pub fn bad(x: u32) -> u32 {
    x - 1
}

#[ply::requires(x > 1)]
#[ply::ensures(|result| *result == x)]
pub fn bad_two(x: u32) -> u32 {
    x - 2
}
