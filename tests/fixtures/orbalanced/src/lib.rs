//! The other direction of the branch-decided measurement fixture
//! (CLAUDE.md, 2026-09-02): a promise whose `||` arms are genuinely
//! balanced must still print the split, but must not be marked -- the mark
//! exists for a lopsided promise, not for every promise shaped as `||`.
//!
//! `thirds`'s postcondition is a three-way tautology over `x % 3`, so each
//! arm decides roughly a third of the generated cases: no single side does
//! "almost all of the deciding" the way a real narrow promise does.
#[ply::ensures(|result| x % 3 == 0 || x % 3 == 1 || x % 3 == 2)]
pub fn thirds(x: u32) -> u32 {
    x
}
