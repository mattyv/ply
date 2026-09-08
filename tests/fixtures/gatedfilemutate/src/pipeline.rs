#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn count_row(x: u32) -> u32 {
    x + 1
}
