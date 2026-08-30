//! A function slow enough that `fuzz`'s cases cannot finish inside a short
//! engine budget, paired with a worked example that fails immediately.
//!
//! `test` and `fuzz` share one cargo subprocess and one deadline, so this is
//! the shape where a real, already-reported failure can be outlived by a
//! sibling check that is still running when the clock runs out.

#[ply::ensures(|result| *result <= 255)]
pub fn slow_identity(x: u8) -> u8 {
    std::thread::sleep(std::time::Duration::from_millis(400));
    x
}
