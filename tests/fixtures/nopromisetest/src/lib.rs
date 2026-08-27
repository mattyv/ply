//! The eleventh false pass, "one step milder"
//! (docs/review-strings-receivers.md finding 1): a function with no
//! `#[ply::ensures]` at all and no `examples:` entries, declared with only
//! the `test` check. There is nothing here for `test` to assert -- no
//! contract, no worked example -- so its harness module used to contribute
//! no test either, and this reported `tested` with zero cases run, same as
//! the receiver-method shape.

pub fn seven() -> u32 {
    7
}
