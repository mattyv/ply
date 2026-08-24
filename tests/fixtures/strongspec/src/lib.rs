//! M4 acceptance fixture: a real, exact spec (`requires` keeps the domain
//! overflow-free, `ensures` pins the exact sum) that `fuzz` + `test`
//! together kill essentially every mutant of -- earning `·spec-strong`
//! (D12: `test`/`fuzz` are `mutate`'s kill signal).
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

#[ply::requires(x < 1000 && y < 1000)]
#[ply::ensures(|result| *result == x + y)]
pub fn add_small(x: u32, y: u32) -> u32 {
    x + y
}
