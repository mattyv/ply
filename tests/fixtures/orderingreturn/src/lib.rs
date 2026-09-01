//! docs/reach-measurement-2.md / The-Ply-Spec.md §5.4b (measured
//! 2026-09-01): a function whose *return* type Ply's codegen never
//! constructs -- the real call produces it -- used to be refused outright
//! on that basis alone, even though nothing in either engine's codegen
//! ever names or constructs a return type. `std::cmp::Ordering` is a type
//! Ply models nowhere, and is exactly the shape that was measured.

use std::cmp::Ordering;

#[ply::ensures(|result| *result != Ordering::Greater || a > b)]
pub fn compare(a: u32, b: u32) -> Ordering {
    a.cmp(&b)
}
