//! Defect 1 (2026-08-30, regression: "a promise nobody checks is now
//! reported green in total silence"): `seven`'s contract is written
//! entirely in `ply.yaml` (`requires:`/`ensures:`), and it also declares
//! `checks: [test]` with an `examples:` entry. The example passes, so
//! `test` reports `tested` -- but nothing ever checks `seven` against the
//! ply.yaml `ensures` clause below, which claims the wrong answer
//! (`result == 99`). A run must say so.

pub fn seven() -> u32 {
    7
}
