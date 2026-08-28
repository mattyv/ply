//! Acceptance fixture for the four shapes added 2026-08-27: `usize`/`isize`,
//! the `NonZero` family, and `Duration` -- the types the rate-limiter
//! measurement (tests/fixtures/ratelimiter/) found dominate ordinary Rust's
//! public surface. Every function here holds for its *entire* domain (a
//! genuinely `bounded`/`fuzzed` proof, never a seeded violation) -- this
//! fixture's whole job is proving these types are now accepted where they
//! were refused before, on both engines.

use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Duration;

/// `usize`, plain and pointer-width -- §5.4b already calls every other
/// integer width "cheap unconditionally"; this is the same claim for the
/// width the standard library uses for lengths and indices everywhere.
#[ply::requires(len < 1_000)]
#[ply::ensures(|result| *result == len + 1)]
pub fn bump_len(len: usize) -> usize {
    len + 1
}

/// `isize`, the signed pointer-width counterpart.
#[ply::requires(delta > -1_000 && delta < 1_000)]
#[ply::ensures(|result| *result == delta * 2)]
pub fn double_delta(delta: isize) -> isize {
    delta * 2
}

/// `NonZeroU32` -- the rate limiter's own dominant non-integer type
/// (`RefillRate`/`Quota`/`TokenBucket`/`KeyedRateLimiter` all take one for
/// "how many tokens"). `.get()` is always `> 0`, by the type's own
/// invariant -- if the generated harness ever let a zero through, this
/// would be the first thing to go red, silently, without the dedicated
/// invariant tests in `crates/ply-core/src/harness.rs`.
#[ply::ensures(|result| *result > 0)]
pub fn tokens_requested(n: NonZeroU32) -> u32 {
    n.get()
}

/// `NonZeroUsize` -- the same invariant, at the width the rate limiter uses
/// for its own key-table caps (`KeyedLimiterConfig::max_keys`/`shard_count`).
#[ply::ensures(|result| *result > 0)]
pub fn shard_count(n: NonZeroUsize) -> usize {
    n.get()
}

/// `Duration` -- the rate limiter's single most common type on its public
/// surface. `subsec_nanos()` is always under one billion by the type's own
/// invariant, for *every* `Duration` the standard library can construct,
/// which is exactly the domain a `bounded` proof over this shape claims to
/// cover.
#[ply::ensures(|result| result.subsec_nanos() < 1_000_000_000)]
pub fn round_trip(d: Duration) -> Duration {
    d
}

/// Two `Duration`s and the requirement that toggling only the affected part
/// keeps the postcondition -- the closest this fixture comes to the rate
/// limiter's own `RefillRate`/`Quota` shape without needing a struct.
#[ply::requires(secs < 1_000_000)]
#[ply::ensures(|result| result.as_secs() == secs)]
pub fn from_whole_seconds(secs: u64) -> Duration {
    Duration::new(secs, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_len_examples() {
        assert_eq!(bump_len(0), 1);
        assert_eq!(bump_len(41), 42);
    }

    #[test]
    fn tokens_requested_is_never_zero() {
        assert_eq!(tokens_requested(NonZeroU32::new(7).unwrap()), 7);
    }

    #[test]
    fn round_trip_is_the_identity() {
        assert_eq!(round_trip(Duration::from_millis(1500)), Duration::from_millis(1500));
    }
}
