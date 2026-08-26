//! §2.2 Quota and configuration — how fast a bucket refills and how big it is.

use std::num::NonZeroU32;
use std::time::Duration;

/// How fast a bucket refills, expressed as whole tokens over an interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefillRate {
    tokens: NonZeroU32,
    per: Duration,
}

impl RefillRate {
    /// `tokens` tokens are added every `per`. Fails if `per` is zero, since
    /// that describes an infinite rate rather than a real one.
    pub fn new(tokens: NonZeroU32, per: Duration) -> Result<Self, ConfigError> {
        if per == Duration::ZERO {
            return Err(ConfigError::ZeroRefillInterval);
        }
        Ok(Self { tokens, per })
    }

    pub fn per_second(tokens: NonZeroU32) -> Self {
        Self { tokens, per: Duration::from_secs(1) }
    }

    pub fn tokens(&self) -> NonZeroU32 {
        self.tokens
    }

    pub fn interval(&self) -> Duration {
        self.per
    }

    pub(crate) fn tokens_per_nanosecond(&self) -> f64 {
        self.tokens.get() as f64 / self.per.as_nanos() as f64
    }
}

/// A complete description of one bucket's shape: how many tokens it can
/// hold at once (the burst it can absorb), and how quickly spent tokens
/// come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    capacity: NonZeroU32,
    refill: RefillRate,
}

impl Quota {
    pub fn new(capacity: NonZeroU32, refill: RefillRate) -> Self {
        Self { capacity, refill }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity.get()
    }

    pub fn refill(&self) -> RefillRate {
        self.refill
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("refill interval must be non-zero")]
    ZeroRefillInterval,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nz(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    #[test]
    fn refill_rate_rejects_zero_interval() {
        let err = RefillRate::new(nz(1), Duration::ZERO).unwrap_err();
        assert_eq!(err, ConfigError::ZeroRefillInterval);
    }

    #[test]
    fn refill_rate_accepts_nonzero_interval() {
        let rate = RefillRate::new(nz(5), Duration::from_secs(1)).unwrap();
        assert_eq!(rate.tokens(), nz(5));
        assert_eq!(rate.interval(), Duration::from_secs(1));
    }

    #[test]
    fn per_second_constructs_a_one_second_interval() {
        let rate = RefillRate::per_second(nz(10));
        assert_eq!(rate.interval(), Duration::from_secs(1));
        assert_eq!(rate.tokens(), nz(10));
    }

    #[test]
    fn tokens_per_nanosecond_matches_the_configured_rate() {
        let rate = RefillRate::per_second(nz(1_000_000_000));
        assert!((rate.tokens_per_nanosecond() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn quota_exposes_capacity_and_refill() {
        let refill = RefillRate::per_second(nz(1));
        let quota = Quota::new(nz(42), refill);
        assert_eq!(quota.capacity(), 42);
        assert_eq!(quota.refill(), refill);
    }

    #[test]
    fn config_error_message_names_the_problem() {
        assert_eq!(ConfigError::ZeroRefillInterval.to_string(), "refill interval must be non-zero");
    }
}
