//! Fixture for the compiler-probe fix (external review, 2026-08-30):
//! `rustc_identity` used to run in whoever's shell happened to invoke
//! `cargo-ply`, not in the crate being verified, so the recorded compiler
//! could be wrong in either direction. This crate carries no
//! `rust-toolchain.toml` of its own -- the e2e test that uses it writes one
//! into a fresh copy and changes it between two runs, so the crate's own
//! compiler moves without the test process's own directory ever changing.

/// One ordinary claim, checked by `test` alone -- no engine-specific
/// behaviour needed, just something that genuinely compiles, runs, and
/// earns a real verdict.
#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn safe_increment(x: u32) -> u32 {
    x + 1
}
