//! M4 acceptance fixture: a vacuous `ensures` (`true`, unconditionally) --
//! it can never fail no matter what the body does, so `mutate` must find
//! every mutant surviving and flag `W0502` weak spec. §1's fourth empirical
//! finding, made concrete: "machine-written specs go vacuous... mutation
//! testing is therefore a first-class check."
//!
//! Pristine "before" state, same convention as the other M3/M4 fixtures.

#[ply::ensures(|_result| true)]
pub fn vacuous(x: u32) -> u32 {
    x.wrapping_add(1)
}
