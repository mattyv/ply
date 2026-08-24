//! Vec<u8> fixture -- exercises the mandatory unwind emission (§5.4b).
//! `checks: [bounded(8)]` in ply.yaml. Measured (not inferred, not copied
//! from the scale spike's own number for a different harness shape): for
//! this exact manual-indexed-loop-over-`any_vec::<u8,8>` shape, unwind=8
//! (== N) fails with "unwinding assertion loop 0" and unwind=9 (== N+1)
//! verifies. See docs/m3-slice-findings.md for the full sweep (5, 8, 9, 16,
//! 22, 24 all tried) and why this number differs from §5.4b's own "22"
//! figure for a bare `any_vec` construction with no consuming loop.
//!
//! Pristine "before" state, same convention as the clamp fixture.

#[ply::ensures(|result| *result <= 255u32 * v.len() as u32)]
pub fn vec_sum(v: &Vec<u8>) -> u32 {
    let mut acc: u32 = 0;
    for i in 0..v.len() {
        acc += v[i] as u32;
    }
    acc
}
