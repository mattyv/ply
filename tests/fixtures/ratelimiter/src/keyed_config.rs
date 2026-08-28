//! §2.2 (continued) — configuration for the per-key limiter, including how
//! it bounds its own memory.

use std::num::NonZeroUsize;
use std::time::Duration;

use crate::quota::Quota;

/// What happens when `max_keys` is set and a request for a brand-new key
/// arrives while the table is already full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Evict the least-recently-touched key and admit the new one instead.
    /// "Least recently touched" is tracked per internal shard, not globally
    /// across the whole table, so under very uneven load across keys this
    /// is an approximation of true LRU, not an exact one.
    ApproximateLru,
    /// Refuse the new key outright. `check` reports it as `Denied` with a
    /// `retry_after` of zero — meaning "not admitted right now, and this
    /// limiter has no ETA for when that will change" — rather than
    /// evicting someone else to make room.
    RejectNewKeys,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyedLimiterConfig {
    pub quota: Quota,
    /// Keys idle for at least this long become eligible for removal by
    /// `sweep`. `None` means keys are kept forever (fine for a bounded key
    /// space; not recommended for keys derived from untrusted input).
    pub idle_eviction: Option<Duration>,
    /// Hard cap on distinct keys tracked at once. `None` means unbounded.
    pub max_keys: Option<NonZeroUsize>,
    pub eviction_policy: EvictionPolicy,
    /// Number of internal shards the key space is split across, for
    /// concurrent access. Higher reduces lock contention under many
    /// concurrent keys; it does not change limiter behavior.
    pub shard_count: NonZeroUsize,
}

impl KeyedLimiterConfig {
    /// Sensible defaults: no idle eviction, no key cap, 16 shards. Adjust
    /// the fields directly afterwards if you need bounded memory use.
    pub fn new(quota: Quota) -> Self {
        Self {
            quota,
            idle_eviction: None,
            max_keys: None,
            eviction_policy: EvictionPolicy::ApproximateLru,
            shard_count: NonZeroUsize::new(16).unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::RefillRate;
    use std::num::NonZeroU32;

    fn quota() -> Quota {
        Quota::new(NonZeroU32::new(5).unwrap(), RefillRate::per_second(NonZeroU32::new(1).unwrap()))
    }

    #[test]
    fn defaults_are_unbounded_with_sixteen_shards() {
        let cfg = KeyedLimiterConfig::new(quota());
        assert_eq!(cfg.idle_eviction, None);
        assert_eq!(cfg.max_keys, None);
        assert_eq!(cfg.eviction_policy, EvictionPolicy::ApproximateLru);
        assert_eq!(cfg.shard_count.get(), 16);
    }

    #[test]
    fn fields_are_directly_adjustable() {
        let mut cfg = KeyedLimiterConfig::new(quota());
        cfg.max_keys = Some(std::num::NonZeroUsize::new(100).unwrap());
        cfg.eviction_policy = EvictionPolicy::RejectNewKeys;
        assert_eq!(cfg.max_keys.unwrap().get(), 100);
        assert_eq!(cfg.eviction_policy, EvictionPolicy::RejectNewKeys);
    }
}
