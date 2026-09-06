//! Two functions with the same narrow precondition, and both promises are
//! **true**. Nothing here is broken, which is the whole point.
//!
//! Ply generates boundary values for a `u32` -- 0, 1, the maximum, and so
//! on -- and none of them is 42. Before 2026-09-06 that produced a
//! **violation** on both: "a real, reproduced violation, not a
//! probabilistic one", about code that keeps its promise perfectly. A false
//! accusation about the author's code is the one failure this project
//! exists to prevent, and it was worse than the gap it had been added to
//! close.
//!
//! The two differ in one thing only: `only_at_42_with_example` has a worked
//! example that satisfies the precondition, so a real input does reach it
//! and it earns evidence. `only_at_42` has none, so nothing reached it and
//! the honest report is that it was never checked -- not that it failed.

#[ply::requires(x == 42)]
#[ply::ensures(|result| *result == x)]
pub fn only_at_42(x: u32) -> u32 {
    x
}

#[ply::requires(x == 42)]
#[ply::ensures(|result| *result == x)]
pub fn only_at_42_with_example(x: u32) -> u32 {
    x
}
