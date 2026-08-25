//! Timeout fixture -- the scale spike's own confound, reused deliberately
//! (SCALE-FINDINGS.md item 1): an iterator-chain body over a `Vec<u8>`
//! times out CBMC even at length 1, because CBMC symbolically unwinds the
//! *generic* `Iterator::fold`/`Map::map_fold` trait-dispatch machinery, not
//! a loop bounded by the real (tiny) length. Ply's `#[kani::unwind]`
//! emission targets the vec-construction/consumption loop shape and does
//! not fix this idiom -- so this fixture must report status `timeout`,
//! never `violation`, and must carry no witness.
//!
//! Pristine "before" state, same convention as the clamp fixture.

#[ply::ensures(|result| *result <= 255u32 * v.len() as u32)]
pub fn vec_iter_sum(v: &Vec<u8>) -> u32 {
    v.iter().map(|&x| x as u32).sum()
}
