//! M4 acceptance fixture: `mutate` declared with no `test`/`fuzz` entry in
//! the same checks list -- D12's own MUST, `E0504`. `mutate` has no kill
//! signal of its own; this must be caught as a config error before any
//! engine runs, not attempted and left to fail confusingly.
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

#[ply::ensures(|result| *result == x)]
pub fn lonely(x: u32) -> u32 {
    x
}
