//! A file module of first-party legacy code. No `ply::` attributes on
//! anything in it -- its contract lives in ply.yaml, keyed by the path a
//! reader of the crate would write: `rates::legacy_rate`.

pub fn legacy_rate(tier: u8) -> u32 {
    if tier == 0 { 150 } else { 90 }
}
