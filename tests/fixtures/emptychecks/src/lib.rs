//! Two identical contracted functions, differing only in what their
//! `ply.yaml` entries say about checking them: one writes `checks: []`, the
//! other writes no `checks:` line at all.
//!
//! An empty list reads to a person as "do not check this". `verify` treated
//! it as *no* list and applied the shape-aware default, so the function was
//! proved anyway -- while `check` and the diagram read the same line as
//! claiming nothing.

#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn declared_unchecked(x: u32) -> u32 {
    x + 1
}

#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn left_to_the_default(x: u32) -> u32 {
    x + 1
}
