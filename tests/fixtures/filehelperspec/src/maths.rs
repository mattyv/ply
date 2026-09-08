/// Where the logic actually lives. `cargo mutants` reports its owner as
/// `doubled_then_capped` -- bare, because this file *is* the module.
pub(super) fn doubled_then_capped(x: u32) -> u32 {
    let doubled = x.saturating_mul(2);
    if doubled > 100 { 100 } else { doubled }
}
