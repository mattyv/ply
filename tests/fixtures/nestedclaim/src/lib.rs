//! A claim written inside a *nested* component (The-Ply-Spec.md §5.1:
//! "components: # optional nested components, same shape").
//!
//! `verify` walked only the top level of the component tree, so this
//! function's claim earned no verdict, no diagnostic and no mention at all
//! -- while `cargo ply check` walked the whole tree and reported the same
//! claim as fine. The document looked correct and nothing ran.

#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn safe_increment(x: u32) -> u32 {
    x + 1
}
