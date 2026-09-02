//! Real-world reproduction (2026-09-01, verified by hand against `semver`'s
//! `Version::parse`): a factually false `examples:` entry passes in total
//! silence under `checks: [fuzz(64)]` -- `fuzz` never compiles or runs a
//! declared example at all, only `test` does, and nothing said so. Worse
//! than plain neglect: the run *notices* the example and re-checks because
//! of it (it is read and fingerprinted as part of what this claim depends
//! on), so it reads the example, fingerprints it, and never evaluates it.

#[ply::ensures(|result| *result == x + 1)]
pub fn increment(x: u32) -> u32 {
    x + 1
}
