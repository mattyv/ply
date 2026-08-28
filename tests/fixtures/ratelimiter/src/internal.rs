//! §3 Internal types — not exported; they hold the state the public API
//! operates on, and the refill/debit arithmetic shared between the two
//! public limiter types.

use std::time::Duration;

use crate::clock::Clock;
use crate::decision::Decision;
use crate::quota::Quota;

/// The state of one bucket: how many tokens (possibly fractional, since
/// refill happens continuously rather than in discrete steps) it currently
/// holds, and when that count was last brought up to date.
pub(crate) struct BucketState<I> {
    pub(crate) tokens: f64,
    pub(crate) updated_at: I,
}

/// The per-key equivalent of `BucketState`, with an extra field purely for
/// approximate-LRU bookkeeping: a monotonically increasing counter stamped
/// on every touch (creation or check), so eviction can find "the entry
/// nobody has touched in the longest time" within a shard by a linear scan
/// for the minimum. That scan is O(shard size); it runs only when the shard
/// is at its configured key cap, not on every request.
pub(crate) struct Entry<I> {
    pub(crate) tokens: f64,
    pub(crate) updated_at: I,
    pub(crate) sequence: u64,
}

/// The refill-and-debit arithmetic shared by `TokenBucket` and
/// `KeyedRateLimiter`, so the two public types can't drift into checking
/// slightly different rules. Mutates `tokens` and `updated_at` in place and
/// returns the resulting decision. Assumes `requested <= quota.capacity()`
/// has already been checked by the caller — this function does not repeat
/// that check, since by this point it always holds.
pub(crate) fn refill_and_debit<C: Clock>(
    quota: &Quota,
    clock: &C,
    tokens: &mut f64,
    updated_at: &mut C::Instant,
    now: C::Instant,
    requested: u32,
) -> Decision {
    let elapsed = clock.duration_since(now, *updated_at);
    if elapsed > Duration::ZERO {
        let gained = elapsed.as_nanos() as f64 * quota.refill().tokens_per_nanosecond();
        *tokens = (*tokens + gained).min(quota.capacity() as f64);
    }
    // Only ever move the bookkeeping timestamp forward. If the clock briefly
    // reported an earlier instant than we last saw (elapsed == 0 above), we
    // must not rewind `updated_at` to match it: doing so would let time
    // "catch up" past the original reading once the clock resumes forward
    // progress, minting tokens for a gap that never really elapsed.
    if now > *updated_at {
        *updated_at = now;
    }

    if *tokens >= requested as f64 {
        *tokens -= requested as f64;
        Decision::Allowed { remaining: tokens.floor() as u32 }
    } else {
        let deficit = requested as f64 - *tokens;
        let rate = quota.refill().tokens_per_nanosecond();
        let nanos_needed = (deficit / rate).ceil() as u64;
        Decision::Denied { retry_after: Duration::from_nanos(nanos_needed) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, FakeClock};
    use crate::quota::RefillRate;
    use std::num::NonZeroU32;

    fn nz(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    #[test]
    fn debits_full_amount_when_available() {
        let quota = Quota::new(nz(10), RefillRate::per_second(nz(1)));
        let clock = FakeClock::new();
        let mut tokens = 10.0;
        let mut updated_at = clock.now();
        let now = clock.now();
        let decision = refill_and_debit(&quota, &clock, &mut tokens, &mut updated_at, now, 4);
        assert_eq!(decision, Decision::Allowed { remaining: 6 });
        assert_eq!(tokens, 6.0);
    }

    #[test]
    fn denies_and_reports_retry_after_when_insufficient() {
        let quota = Quota::new(nz(10), RefillRate::per_second(nz(1)));
        let clock = FakeClock::new();
        let mut tokens = 2.0;
        let mut updated_at = clock.now();
        let now = clock.now();
        let decision = refill_and_debit(&quota, &clock, &mut tokens, &mut updated_at, now, 5);
        match decision {
            Decision::Denied { retry_after } => {
                // 3 tokens short at 1 token/sec => 3 seconds.
                assert_eq!(retry_after, Duration::from_secs(3));
            }
            other => panic!("expected Denied, got {other:?}"),
        }
        // Denied must not debit anything.
        assert_eq!(tokens, 2.0);
    }

    #[test]
    fn refill_never_exceeds_capacity() {
        let quota = Quota::new(nz(5), RefillRate::per_second(nz(1)));
        let clock = FakeClock::new();
        let mut tokens = 5.0;
        let mut updated_at = clock.now();
        clock.advance(Duration::from_secs(1_000_000)); // a very long idle gap
        let now = clock.now();
        let decision = refill_and_debit(&quota, &clock, &mut tokens, &mut updated_at, now, 1);
        assert_eq!(decision, Decision::Allowed { remaining: 4 });
        assert!(tokens <= 5.0);
    }

    #[test]
    fn updated_at_never_moves_backwards() {
        let quota = Quota::new(nz(5), RefillRate::per_second(nz(1)));
        let clock = FakeClock::new();
        let mut tokens = 5.0;
        clock.advance(Duration::from_secs(10));
        let mut updated_at = clock.now();
        // Simulate a clock hiccup: `now` reports a moment before `updated_at`.
        let now = Duration::from_secs(5);
        let _ = refill_and_debit(&quota, &clock, &mut tokens, &mut updated_at, now, 1);
        assert_eq!(updated_at, Duration::from_secs(10));
    }

    #[test]
    fn a_backwards_clock_hiccup_costs_zero_elapsed_time_not_negative() {
        let quota = Quota::new(nz(5), RefillRate::per_second(nz(1)));
        let clock = FakeClock::new();
        let mut tokens = 2.0;
        clock.advance(Duration::from_secs(10));
        let mut updated_at = clock.now();
        let now = Duration::from_secs(3); // before updated_at
        let decision = refill_and_debit(&quota, &clock, &mut tokens, &mut updated_at, now, 1);
        // No time elapsed (clamped to zero), no refill happened, but the
        // request still succeeds because 2 tokens >= 1 requested.
        assert_eq!(decision, Decision::Allowed { remaining: 1 });
    }
}
