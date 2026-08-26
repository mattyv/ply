//! Fixture for The-Ply-Spec.md §5.2a: a proof descends into a callee that
//! carries its own contract (§5.5's first branch), so that callee's real
//! body is part of what the proof established -- and until 2026-08-25 no
//! part of the recorded fingerprint.

/// Carries its own contract, so §5.5's first branch lets the proof descend
/// into the real body rather than replacing it with a promise.
#[ply::requires(x <= 50)]
#[ply::ensures(|result| *result <= 200)]
pub fn inner(x: u32) -> u32 {
    x * 2
}

/// Proved with `bounded`, which reads `inner`'s body. Editing that body
/// changes what was proved, so it must re-earn the proof.
#[ply::requires(x <= 50)]
#[ply::ensures(|result| *result <= 200)]
pub fn outer(x: u32) -> u32 {
    inner(x)
}
