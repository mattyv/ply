//! `old(...)` on the fuzz/test path (The-Ply-Spec.md §5.4a), and the shape
//! it was introduced for.
//!
//! `old(expr)` is the value `expr` had when the function was entered. The
//! model checker has a primitive for it; a generated proptest/`#[test]`
//! harness does not, so §5.4a prescribes the substitution -- "evaluate
//! `expr` before the call and substitute the snapshot". Until 2026-08-25
//! nothing did it, and `bump` came back as an internal tool error quoting
//! `cannot find function `old` in this scope` from Ply's own generated file.
//!
//! `bump_in_place` is the second half of the story, and it is a refusal
//! rather than a fix: a parameter the function writes back through is not
//! a shape either engine can build (§5.4b stops at a shared `&T`). It used
//! to be read as a plain `u32`, which produced a harness that could not
//! compile; it is now reported as an unsupported shape, by name.

/// Reads its own parameter's entry value. Holds for every input `requires`
/// admits.
#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == old(x) + 1)]
pub fn bump(x: u32) -> u32 {
    x + 1
}

/// The shape `old()` exists for: it changes something in place and returns
/// nothing useful, so a single-state postcondition could not describe it.
#[ply::ensures(|result| *counter == old(*counter) + 1)]
pub fn bump_in_place(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}
