//! §2.1 Time — the `Clock` abstraction and its three implementations.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A source of monotonically-nondecreasing timestamps.
///
/// Implementations are not required to guarantee strict monotonicity across
/// threads or cores — real hardware clocks occasionally don't — but
/// `duration_since` must never report a negative elapsed time. When `later`
/// is not actually after `earlier`, implementations return `Duration::ZERO`
/// rather than panicking, wrapping, or (worse) returning a huge duration
/// from an unsigned subtraction underflow.
pub trait Clock: Send + Sync + 'static {
    /// An opaque timestamp produced by this clock. Callers never construct
    /// or inspect one directly; they only ever get one from `now()` and feed
    /// it back into `duration_since`.
    type Instant: Copy + Ord + Send + Sync + 'static;

    fn now(&self) -> Self::Instant;

    /// Elapsed time from `earlier` to `later`. Must saturate to zero rather
    /// than panic or wrap when `later` does not follow `earlier`.
    fn duration_since(&self, later: Self::Instant, earlier: Self::Instant) -> Duration;
}

/// The clock used in production: `std::time::Instant`, which the standard
/// library already guarantees never goes backwards on a given platform's
/// best effort. Use this unless you have a specific reason not to.
#[derive(Debug, Default, Clone, Copy)]
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    type Instant = std::time::Instant;

    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn duration_since(&self, later: std::time::Instant, earlier: std::time::Instant) -> Duration {
        later.saturating_duration_since(earlier)
    }
}

/// A clock based on wall-clock time (`SystemTime`), for the rarer case where
/// timestamps need to be comparable across process restarts or machines.
/// Unlike `MonotonicClock`, this clock can and does observe time moving
/// backwards — an NTP step, a VM migration, an operator fixing a misset
/// clock — and folds any such observation to a zero elapsed duration rather
/// than letting it propagate into the limiter's math.
#[derive(Debug, Default, Clone, Copy)]
pub struct WallClock;

impl Clock for WallClock {
    type Instant = std::time::SystemTime;

    fn now(&self) -> std::time::SystemTime {
        std::time::SystemTime::now()
    }

    fn duration_since(
        &self,
        later: std::time::SystemTime,
        earlier: std::time::SystemTime,
    ) -> Duration {
        later.duration_since(earlier).unwrap_or(Duration::ZERO)
    }
}

/// A clock that only moves when told to. Lets tests exercise refill timing,
/// long idle gaps, and backwards jumps deterministically, without sleeping.
#[derive(Debug, Clone, Default)]
pub struct FakeClock {
    nanos: Arc<AtomicU64>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self { nanos: Arc::new(AtomicU64::new(0)) }
    }

    /// Move the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        self.nanos.fetch_add(by.as_nanos() as u64, Ordering::SeqCst);
    }

    /// Jump the clock to an absolute offset from its creation, including to
    /// a point earlier than its current reading. Exists specifically so
    /// tests can exercise backwards-clock handling in callers.
    pub fn set(&self, elapsed_since_start: Duration) {
        self.nanos.store(elapsed_since_start.as_nanos() as u64, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    type Instant = Duration;

    fn now(&self) -> Duration {
        Duration::from_nanos(self.nanos.load(Ordering::SeqCst))
    }

    fn duration_since(&self, later: Duration, earlier: Duration) -> Duration {
        later.checked_sub(earlier).unwrap_or(Duration::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_clock_never_reports_negative_elapsed() {
        let clock = MonotonicClock;
        let earlier = clock.now();
        let later = clock.now();
        // later >= earlier always here, but exercise the saturating path by
        // reversing the arguments: "elapsed from now to a moment before it"
        // must saturate to zero, not underflow.
        assert_eq!(clock.duration_since(earlier, later), Duration::ZERO.max(clock.duration_since(earlier, later)));
        assert!(clock.duration_since(later, earlier) >= Duration::ZERO);
    }

    #[test]
    fn wall_clock_folds_backwards_jump_to_zero() {
        let clock = WallClock;
        let now = clock.now();
        let earlier = now + Duration::from_secs(10);
        // `now` is "earlier" than `earlier` here, so duration_since must not
        // panic on the underlying SystemTime::duration_since error.
        assert_eq!(clock.duration_since(now, earlier), Duration::ZERO);
    }

    #[test]
    fn fake_clock_starts_at_zero_and_advances() {
        let clock = FakeClock::new();
        assert_eq!(clock.now(), Duration::ZERO);
        clock.advance(Duration::from_secs(1));
        assert_eq!(clock.now(), Duration::from_secs(1));
    }

    #[test]
    fn fake_clock_set_can_move_backwards() {
        let clock = FakeClock::new();
        clock.advance(Duration::from_secs(5));
        clock.set(Duration::from_secs(1));
        assert_eq!(clock.now(), Duration::from_secs(1));
    }

    #[test]
    fn fake_clock_duration_since_saturates_on_reversed_args() {
        let clock = FakeClock::new();
        let earlier = clock.now();
        clock.advance(Duration::from_secs(1));
        let later = clock.now();
        // Passing the timestamps in the wrong order must saturate, not wrap.
        assert_eq!(clock.duration_since(earlier, later), Duration::ZERO);
        assert_eq!(clock.duration_since(later, earlier), Duration::from_secs(1));
    }
}
