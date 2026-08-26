//! §2.4 Single bucket — the unkeyed limiter used directly for a global
//! limit, and as the model the per-key limiter replicates per key.

use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::Duration;

use crate::clock::{Clock, MonotonicClock};
use crate::decision::Decision;
use crate::internal::{refill_and_debit, BucketState};
use crate::quota::Quota;

pub struct TokenBucket<C: Clock = MonotonicClock> {
    quota: Quota,
    clock: C,
    state: Mutex<BucketState<C::Instant>>,
}

impl<C: Clock> TokenBucket<C> {
    /// A new bucket starts full: the first request against a fresh bucket
    /// is judged only against capacity, never denied for "history" that
    /// doesn't exist.
    pub fn new(quota: Quota, clock: C) -> Self {
        let now = clock.now();
        Self {
            quota,
            clock,
            state: Mutex::new(BucketState { tokens: quota.capacity() as f64, updated_at: now }),
        }
    }

    pub fn quota(&self) -> Quota {
        self.quota
    }

    pub fn check(&self) -> Decision {
        self.check_n(NonZeroU32::new(1).unwrap())
    }

    /// Attempt to admit a request costing `n` tokens (use `n > 1` for
    /// weighted / expensive operations).
    pub fn check_n(&self, n: NonZeroU32) -> Decision {
        let requested = n.get();
        if requested > self.quota.capacity() {
            return Decision::Unsatisfiable { capacity: self.quota.capacity(), requested };
        }

        let now = self.clock.now();
        let mut guard = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // DEVIATION: the design document passes `&mut state.tokens` and
        // `&mut state.updated_at` straight out of the `MutexGuard` in one
        // call. That doesn't compile: each field projection through a
        // `MutexGuard` requires its own `DerefMut::deref_mut` call, and the
        // borrow checker won't treat two such calls as disjoint borrows of
        // `state` (unlike two plain field projections on an already-plain
        // `&mut BucketState`, which it does allow). Deref-ing once into a
        // plain `&mut BucketState` first, then projecting both fields off
        // that, is the standard fix and changes nothing observable.
        let state = &mut *guard;
        refill_and_debit(&self.quota, &self.clock, &mut state.tokens, &mut state.updated_at, now, requested)
    }

    /// How long until `n` tokens would be available, without spending
    /// anything. A snapshot, not a promise: another caller can spend tokens
    /// from this same bucket between this call returning and any later
    /// `check_n`.
    pub fn time_until_ready(&self, n: NonZeroU32) -> Result<Duration, RequestExceedsCapacity> {
        let requested = n.get();
        if requested > self.quota.capacity() {
            return Err(RequestExceedsCapacity { capacity: self.quota.capacity(), requested });
        }

        let now = self.clock.now();
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let elapsed = self.clock.duration_since(now, state.updated_at);
        let projected = (state.tokens
            + elapsed.as_nanos() as f64 * self.quota.refill().tokens_per_nanosecond())
        .min(self.quota.capacity() as f64);

        if projected >= requested as f64 {
            Ok(Duration::ZERO)
        } else {
            let deficit = requested as f64 - projected;
            let rate = self.quota.refill().tokens_per_nanosecond();
            Ok(Duration::from_nanos((deficit / rate).ceil() as u64))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestExceedsCapacity {
    pub capacity: u32,
    pub requested: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use crate::quota::RefillRate;

    fn nz(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    fn quota(capacity: u32, per_sec: u32) -> Quota {
        Quota::new(nz(capacity), RefillRate::per_second(nz(per_sec)))
    }

    #[test]
    fn a_fresh_bucket_starts_full() {
        let bucket = TokenBucket::new(quota(5, 1), FakeClock::new());
        assert_eq!(bucket.check_n(nz(5)), Decision::Allowed { remaining: 0 });
    }

    #[test]
    fn check_debits_one_token() {
        let bucket = TokenBucket::new(quota(5, 1), FakeClock::new());
        assert_eq!(bucket.check(), Decision::Allowed { remaining: 4 });
    }

    #[test]
    fn over_capacity_request_is_unsatisfiable_not_denied() {
        let bucket = TokenBucket::new(quota(5, 1), FakeClock::new());
        assert_eq!(
            bucket.check_n(nz(6)),
            Decision::Unsatisfiable { capacity: 5, requested: 6 }
        );
    }

    #[test]
    fn time_until_ready_is_zero_when_tokens_available() {
        let bucket = TokenBucket::new(quota(5, 1), FakeClock::new());
        assert_eq!(bucket.time_until_ready(nz(3)).unwrap(), Duration::ZERO);
    }

    #[test]
    fn time_until_ready_reports_over_capacity_as_an_error() {
        let bucket = TokenBucket::new(quota(5, 1), FakeClock::new());
        let err = bucket.time_until_ready(nz(6)).unwrap_err();
        assert_eq!(err, RequestExceedsCapacity { capacity: 5, requested: 6 });
    }

    #[test]
    fn time_until_ready_does_not_spend_tokens() {
        let bucket = TokenBucket::new(quota(5, 1), FakeClock::new());
        let _ = bucket.time_until_ready(nz(5)).unwrap();
        // Still full: a full-capacity check should succeed afterwards.
        assert_eq!(bucket.check_n(nz(5)), Decision::Allowed { remaining: 0 });
    }

    #[test]
    fn refill_restores_tokens_over_time() {
        let clock = FakeClock::new();
        let bucket = TokenBucket::new(quota(5, 1), clock.clone());
        bucket.check_n(nz(5)); // drain it
        clock.advance(Duration::from_secs(2));
        assert_eq!(bucket.check(), Decision::Allowed { remaining: 1 });
    }

    #[test]
    fn denied_reports_a_retry_after_that_then_succeeds() {
        let clock = FakeClock::new();
        let bucket = TokenBucket::new(quota(1, 1), clock.clone());
        bucket.check(); // drain the single token
        let decision = bucket.check();
        let retry_after = match decision {
            Decision::Denied { retry_after } => retry_after,
            other => panic!("expected Denied, got {other:?}"),
        };
        clock.advance(retry_after);
        assert!(bucket.check().is_allowed());
    }

    #[test]
    fn quota_accessor_returns_the_configured_quota() {
        let q = quota(5, 1);
        let bucket = TokenBucket::new(q, FakeClock::new());
        assert_eq!(bucket.quota(), q);
    }
}
