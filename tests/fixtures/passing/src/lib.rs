//! Passing fixture -- a contract that holds, exercising a clean `bounded(k)`
//! verdict (not only the falsified case). `checks: [bounded(2)]` in
//! ply.yaml. Pristine "before" state, same convention as the clamp fixture.

#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn safe_increment(x: u32) -> u32 {
    x + 1
}
