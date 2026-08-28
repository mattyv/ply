//! docs/review-silent-narrowing.md finding 1, 2026-08-28: `Till`'s only
//! mutating operation, `take`, is added via a *second* `impl Till` block in
//! a different file (`more.rs`) than the one `Till` itself is declared in
//! (`till.rs`). Before this fixture's fix, the receiver scan read the
//! declaring file only, so `take` never entered `Till::total`'s operation
//! pool -- every generated case called `total` on a receiver only the
//! constructor had ever touched, and `total`'s promise (always 0) reported
//! a clean `fuzzed(n)` pass forever, even though the real program breaks it
//! after one ordinary call:
//!
//! ```ignore
//! let mut t = Till::new();
//! t.take(5);
//! t.total() // -> 5, not 0
//! ```
//!
//! An ordinary second `impl` block in a second file is completely normal
//! Rust; this fixture is the fix's own acceptance case that the scan now
//! opens it.
pub mod till;
pub mod more;
