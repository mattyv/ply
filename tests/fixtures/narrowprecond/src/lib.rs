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

/// Two parameters, and the precondition needs a *specific pair*. The
/// example supplies it; both values have to arrive in the same call for
/// anything to reach the body at all (external review, 2026-09-06).
#[ply::requires(x == 42 && flag)]
#[ply::ensures(|result| *result == x)]
pub fn only_at_42_and_true(x: u32, flag: bool) -> u32 {
    let _ = flag;
    x
}

/// The promise is **broken**: it says the result is zero and returns one.
///
/// No generated boundary value satisfies `x == 42`, and the worked example
/// asserts only its own expression -- `broken_but_exampled(42) == 1` is a
/// true statement about a function whose contract is false. From the fix
/// for the false accusation (2026-09-06 morning) until that evening, a
/// passing example was read as proof the contract had been checked, and
/// this came back `tested`, exit 0. That is evidence that lies, which is
/// the one thing this tool exists not to do.
///
/// The example's *input* now feeds the generated contract cases, so the
/// promise is asserted at 42 and this is a reported violation.
#[ply::requires(x == 42)]
#[ply::ensures(|result| *result == 0)]
pub fn broken_but_exampled(x: u32) -> u32 {
    let _ = x;
    1
}
