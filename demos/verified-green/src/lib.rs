//! A tiny crate whose promises Ply can actually check, so the drawing has
//! something real to colour.

/// Clamps a reading into the meter's range.
#[ply::requires(lo <= hi)]
#[ply::ensures(|result| *result >= lo && *result <= hi)]
pub fn clamp(value: u32, lo: u32, hi: u32) -> u32 {
    if value < lo { lo } else if value > hi { hi } else { value }
}

/// How many digits are in a label.
#[ply::ensures(|result| *result <= text.len())]
pub fn digit_count(text: &str) -> usize {
    text.chars().filter(|c| c.is_ascii_digit()).count()
}

/// Total of two readings, saturating rather than wrapping.
#[ply::ensures(|result| *result >= a && *result >= b)]
pub fn total(a: u32, b: u32) -> u32 { a.saturating_add(b) }

// Ply-generated module declaration -- do not edit this line.
mod ply_generated;
