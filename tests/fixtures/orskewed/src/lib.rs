//! Regression fixture for the confident-verdict defect this task fixes
//! (CLAUDE.md, 2026-09-02): a promise written as a top-level `||` can be
//! satisfied almost entirely by one side of it, while the run still prints
//! an unqualified `fuzzed(n)`. Modelled directly on the real defect found
//! pointing Ply at `semver`'s `Version::parse`, whose own promise is
//! `!text.contains(' ') || result.is_err()` -- the same "guard decides
//! almost everything, the real rule barely runs" shape, without the
//! `&str`/parsing machinery that shape needs to reproduce.
//!
//! `maybe_pass_through`'s own guard is `x < 100`, and Ply's int generator
//! draws 3-in-4 of its values from 0..=16 (§10 M4: "ints biased small") --
//! every one of those is `< 100`, so the guard decides almost every case,
//! and the promise's other half (`result.unwrap() == x`) only ever runs on
//! the rare draw that lands at 100 or above.
//!
//! This fixture doubles as the short-circuit proof CLAUDE.md's first trap
//! requires: when `x < 100`, `maybe_pass_through` returns `None`, so if the
//! generated check ever forced `result.unwrap()` on that branch instead of
//! short-circuiting past it the way `||` itself would, this fixture would
//! panic instead of holding -- proving evaluation order was preserved,
//! not merely asserted.
#[ply::ensures(|result| x < 100 || result.unwrap() == x)]
pub fn maybe_pass_through(x: u32) -> Option<u32> {
    if x < 100 { None } else { Some(x) }
}
