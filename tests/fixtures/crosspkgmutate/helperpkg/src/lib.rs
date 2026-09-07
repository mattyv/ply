//! Where the logic lives, in a package of its own.

/// Doubles, then caps. The claimed function in the crate next door is a
/// thin shell over this, and this is the body a deliberate bug has to
/// reach for `spec-strong` to mean anything.
pub fn doubled_then_capped(x: u32) -> u32 {
    let doubled = x.saturating_mul(2);
    if doubled > 100 { 100 } else { doubled }
}
