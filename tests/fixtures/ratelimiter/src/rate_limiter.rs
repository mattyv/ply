//! §2.6 A shared trait for callers who don't care which kind they have.

use std::hash::{BuildHasher, Hash};
use std::num::NonZeroU32;

use crate::bucket::TokenBucket;
use crate::clock::Clock;
use crate::decision::Decision;
use crate::keyed::KeyedRateLimiter;

/// Common interface over "something that admits or denies requests",
/// implemented by both `TokenBucket` (with `Key = ()`) and
/// `KeyedRateLimiter` (with `Key = K`). Lets middleware and other generic
/// call sites be written once against either.
pub trait RateLimiter {
    type Key: ?Sized;

    fn check(&self, key: &Self::Key) -> Decision {
        self.check_n(key, NonZeroU32::new(1).unwrap())
    }

    fn check_n(&self, key: &Self::Key, n: NonZeroU32) -> Decision;
}

impl<C: Clock> RateLimiter for TokenBucket<C> {
    type Key = ();

    fn check_n(&self, _key: &(), n: NonZeroU32) -> Decision {
        TokenBucket::check_n(self, n)
    }
}

impl<K, C, S> RateLimiter for KeyedRateLimiter<K, C, S>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    C: Clock,
    S: BuildHasher + Clone,
{
    type Key = K;

    fn check_n(&self, key: &K, n: NonZeroU32) -> Decision {
        KeyedRateLimiter::check_n(self, key, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use crate::keyed_config::KeyedLimiterConfig;
    use crate::quota::{Quota, RefillRate};
    use std::num::NonZeroU32;

    fn nz(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    fn quota() -> Quota {
        Quota::new(nz(3), RefillRate::per_second(nz(1)))
    }

    /// A call site written once against the trait, used with either
    /// concrete limiter — this is the whole point of §2.6.
    fn probe<L: RateLimiter>(limiter: &L, key: &L::Key) -> Decision {
        limiter.check(key)
    }

    #[test]
    fn token_bucket_implements_rate_limiter_with_unit_key() {
        let bucket = TokenBucket::new(quota(), FakeClock::new());
        assert_eq!(probe(&bucket, &()), Decision::Allowed { remaining: 2 });
    }

    #[test]
    fn keyed_rate_limiter_implements_rate_limiter_with_its_own_key_type() {
        let limiter: KeyedRateLimiter<&str, _> =
            KeyedRateLimiter::new(KeyedLimiterConfig::new(quota()), FakeClock::new());
        assert_eq!(probe(&limiter, &"a"), Decision::Allowed { remaining: 2 });
    }

    #[test]
    fn trait_default_check_matches_check_n_of_one() {
        // Two identically-configured, independent buckets: the trait's
        // default `check` on one must agree with `check_n(1)` on the other.
        let a = TokenBucket::new(quota(), FakeClock::new());
        let b = TokenBucket::new(quota(), FakeClock::new());
        assert_eq!(RateLimiter::check(&a, &()), b.check_n(nz(1)));
    }
}
