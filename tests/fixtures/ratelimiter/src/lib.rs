//! Flowgate: a token-bucket rate limiter.
//!
//! This crate is a from-scratch, faithful build of
//! `docs/greenfield-ratelimiter-design.md` (Flowgate), assembled as a Ply
//! fixture to measure verification coverage against naturally-designed
//! code. See `INVARIANTS.md` in this directory for the eleven invariants
//! the design document states, numbered, with the functions each one
//! constrains.
//!
//! Flowgate decides, on every incoming request, whether to let it through
//! right now, and if not, how long the caller should wait before trying
//! again. It supports a single global limit (`TokenBucket`) as well as an
//! independent limit per key (`KeyedRateLimiter`), and separates "what time
//! is it" (`Clock`) from the rest of the logic so that limiter behavior can
//! be tested without sleeping in a test suite.
//!
//! Module layout is an implementation detail not specified by the design
//! document (which describes only the public API, flat); every public item
//! below is re-exported at the crate root so callers use it exactly as
//! documented, e.g. `flowgate::TokenBucket`, `flowgate::Decision`.

mod bucket;
mod clock;
mod decision;
mod internal;
mod keyed;
mod keyed_config;
mod quota;
mod rate_limiter;

pub use bucket::{RequestExceedsCapacity, TokenBucket};
pub use clock::{Clock, FakeClock, MonotonicClock, WallClock};
pub use decision::Decision;
pub use keyed::{KeyedRateLimiter, SweepReport};
pub use keyed_config::{EvictionPolicy, KeyedLimiterConfig};
pub use quota::{ConfigError, Quota, RefillRate};
pub use rate_limiter::RateLimiter;

#[cfg(test)]
mod integration_tests {
    //! End-to-end tests exercising the public surface the way an
    //! application would, across module boundaries.

    use super::*;
    use std::num::NonZeroU32;
    use std::time::Duration;

    fn nz(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    #[test]
    fn a_global_limiter_admits_up_to_capacity_then_denies() {
        let quota = Quota::new(nz(3), RefillRate::per_second(nz(1)));
        let bucket = TokenBucket::new(quota, FakeClock::new());
        assert!(bucket.check().is_allowed());
        assert!(bucket.check().is_allowed());
        assert!(bucket.check().is_allowed());
        assert!(!bucket.check().is_allowed());
    }

    #[test]
    fn a_per_key_limiter_tracks_users_independently() {
        let quota = Quota::new(nz(2), RefillRate::per_second(nz(1)));
        let cfg = KeyedLimiterConfig::new(quota);
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());

        assert!(limiter.check(&"alice").is_allowed());
        assert!(limiter.check(&"alice").is_allowed());
        assert!(!limiter.check(&"alice").is_allowed()); // alice is throttled

        // bob is unaffected by alice's usage.
        assert!(limiter.check(&"bob").is_allowed());
    }

    #[test]
    fn denied_retry_after_is_honored_by_a_real_wait() {
        let clock = FakeClock::new();
        let quota = Quota::new(nz(1), RefillRate::per_second(nz(1)));
        let bucket = TokenBucket::new(quota, clock.clone());

        bucket.check(); // drain
        let Decision::Denied { retry_after } = bucket.check() else {
            panic!("expected Denied");
        };
        clock.advance(retry_after);
        assert!(bucket.check().is_allowed());
    }

    #[test]
    fn a_config_error_prevents_constructing_an_infinite_rate() {
        let result = RefillRate::new(nz(1), Duration::ZERO);
        assert!(matches!(result, Err(ConfigError::ZeroRefillInterval)));
    }

    #[test]
    fn a_request_bigger_than_capacity_is_unsatisfiable_on_both_limiter_kinds() {
        let quota = Quota::new(nz(5), RefillRate::per_second(nz(1)));
        let bucket = TokenBucket::new(quota, FakeClock::new());
        assert_eq!(
            bucket.check_n(nz(6)),
            Decision::Unsatisfiable { capacity: 5, requested: 6 }
        );

        let limiter: KeyedRateLimiter<&str, _> =
            KeyedRateLimiter::new(KeyedLimiterConfig::new(quota), FakeClock::new());
        assert_eq!(
            limiter.check_n(&"x", nz(6)),
            Decision::Unsatisfiable { capacity: 5, requested: 6 }
        );
    }

    #[test]
    fn generic_middleware_works_against_either_limiter_via_the_trait() {
        fn admit<L: RateLimiter>(limiter: &L, key: &L::Key) -> bool {
            limiter.check(key).is_allowed()
        }

        let quota = Quota::new(nz(1), RefillRate::per_second(nz(1)));
        let bucket = TokenBucket::new(quota, FakeClock::new());
        assert!(admit(&bucket, &()));

        let limiter: KeyedRateLimiter<&str, _> =
            KeyedRateLimiter::new(KeyedLimiterConfig::new(quota), FakeClock::new());
        assert!(admit(&limiter, &"x"));
    }

    #[test]
    fn wall_clock_and_monotonic_clock_are_usable_as_the_generic_clock() {
        let quota = Quota::new(nz(5), RefillRate::per_second(nz(1)));
        let mono = TokenBucket::new(quota, MonotonicClock);
        assert!(mono.check().is_allowed());
        let wall = TokenBucket::new(quota, WallClock);
        assert!(wall.check().is_allowed());
    }
}
