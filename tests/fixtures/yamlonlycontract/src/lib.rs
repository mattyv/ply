//! Defect 2 (2026-08-30): `seven` has no `#[ply::requires]`/`#[ply::ensures]`
//! attribute at all -- its only contract is the `requires:`/`ensures:` pair
//! written in `ply.yaml`, which neither `check` nor `verify` used to
//! disclose is not what actually reaches the checkers.

pub fn seven() -> u32 {
    7
}
