//! §2.5 Per-key limiter — an independent bucket per key (per user ID, per
//! API token, per source IP), sharded for concurrent access.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use crate::clock::{Clock, MonotonicClock};
use crate::decision::Decision;
use crate::internal::{refill_and_debit, Entry};
use crate::keyed_config::{EvictionPolicy, KeyedLimiterConfig};

pub struct KeyedRateLimiter<K, C = MonotonicClock, S = RandomState>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    C: Clock,
    S: BuildHasher + Clone,
{
    config: KeyedLimiterConfig,
    clock: C,
    hash_builder: S,
    #[allow(clippy::type_complexity)]
    shards: Vec<RwLock<HashMap<K, Entry<C::Instant>, S>>>,
    touch_sequence: AtomicU64,
}

pub struct SweepReport {
    pub keys_before: usize,
    pub keys_removed: usize,
    pub keys_after: usize,
}

impl<K, C> KeyedRateLimiter<K, C, RandomState>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    C: Clock,
{
    pub fn new(config: KeyedLimiterConfig, clock: C) -> Self {
        Self::with_hasher(config, clock, RandomState::default())
    }
}

// DEVIATION (see INVARIANTS.md / fixture report): the design document puts
// `with_hasher` and all the other inherent methods (`check_n`, `sweep`,
// `len`, `remove`, ...) in one `impl` block bounded by
// `S: BuildHasher + Clone + Default`, then implements `RateLimiter` for
// `KeyedRateLimiter` with the weaker bound `S: BuildHasher + Clone` (no
// `Default`) and has that impl call `self.check_n(...)`. As written this
// does not type-check: resolving `check_n` on a generic `S` that is only
// known to be `BuildHasher + Clone` requires an inherent impl available
// under those exact bounds, and the document's `Default` bound blocks that.
// Nothing in the method bodies below actually needs `S: Default` --
// `HashMap::with_hasher` takes a hasher value, not a `Default::default()`
// call -- so `Default` here reads as a copy-paste artifact rather than a
// deliberate constraint. The obvious-correct fix is to drop it, which is
// what this impl block does; behavior and every other signature are
// unchanged.
impl<K, C, S> KeyedRateLimiter<K, C, S>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    C: Clock,
    S: BuildHasher + Clone,
{
    pub fn with_hasher(config: KeyedLimiterConfig, clock: C, hash_builder: S) -> Self {
        let shard_count = config.shard_count.get();
        let shards = (0..shard_count)
            .map(|_| RwLock::new(HashMap::with_hasher(hash_builder.clone())))
            .collect();
        Self { config, clock, hash_builder, shards, touch_sequence: AtomicU64::new(0) }
    }

    pub fn check(&self, key: &K) -> Decision {
        self.check_n(key, NonZeroU32::new(1).unwrap())
    }

    /// Attempt to admit a request of weight `n` against `key`'s own bucket.
    /// A key seen for the first time gets a fresh, full bucket — it is
    /// never penalized for the history of any other key, and never starts
    /// out already throttled.
    pub fn check_n(&self, key: &K, n: NonZeroU32) -> Decision {
        let requested = n.get();
        if requested > self.config.quota.capacity() {
            return Decision::Unsatisfiable { capacity: self.config.quota.capacity(), requested };
        }

        let now = self.clock.now();
        let shard = &self.shards[self.shard_index(key)];

        // Fast path: key already tracked.
        {
            let mut map = shard.write().unwrap_or_else(|p| p.into_inner());
            if let Some(entry) = map.get_mut(key) {
                entry.sequence = self.touch_sequence.fetch_add(1, Ordering::Relaxed);
                return refill_and_debit(
                    &self.config.quota,
                    &self.clock,
                    &mut entry.tokens,
                    &mut entry.updated_at,
                    now,
                    requested,
                );
            }
        }

        // Slow path: unseen key. Re-take the write lock and re-check, since
        // another thread may have inserted this same key in between.
        let mut map = shard.write().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = map.get_mut(key) {
            entry.sequence = self.touch_sequence.fetch_add(1, Ordering::Relaxed);
            return refill_and_debit(
                &self.config.quota,
                &self.clock,
                &mut entry.tokens,
                &mut entry.updated_at,
                now,
                requested,
            );
        }

        if let Some(max_keys) = self.config.max_keys {
            if map.len() >= max_keys.get() {
                match self.config.eviction_policy {
                    EvictionPolicy::RejectNewKeys => {
                        return Decision::Denied { retry_after: std::time::Duration::ZERO };
                    }
                    EvictionPolicy::ApproximateLru => {
                        if let Some(stale_key) =
                            map.iter().min_by_key(|(_, e)| e.sequence).map(|(k, _)| k.clone())
                        {
                            map.remove(&stale_key);
                        }
                    }
                }
            }
        }

        let mut entry = Entry {
            tokens: self.config.quota.capacity() as f64,
            updated_at: now,
            sequence: self.touch_sequence.fetch_add(1, Ordering::Relaxed),
        };
        let decision = refill_and_debit(
            &self.config.quota,
            &self.clock,
            &mut entry.tokens,
            &mut entry.updated_at,
            now,
            requested,
        );
        map.insert(key.clone(), entry);
        decision
    }

    /// Remove keys idle for at least `idle_eviction`. A no-op if
    /// `idle_eviction` was not configured. Call this periodically (e.g. from
    /// a background task) in a service that sees an open-ended stream of
    /// keys (raw IPs, anonymous session IDs) to keep memory bounded; a key
    /// removed this way is indistinguishable from one never seen, so the
    /// next request for it simply starts a fresh, full bucket.
    pub fn sweep(&self) -> SweepReport {
        let Some(idle_threshold) = self.config.idle_eviction else {
            let n = self.len();
            return SweepReport { keys_before: n, keys_removed: 0, keys_after: n };
        };

        let now = self.clock.now();
        let mut before = 0;
        for shard in &self.shards {
            let mut map = shard.write().unwrap_or_else(|p| p.into_inner());
            before += map.len();
            map.retain(|_, entry| self.clock.duration_since(now, entry.updated_at) < idle_threshold);
        }
        let after = self.len();
        SweepReport { keys_before: before, keys_removed: before - after, keys_after: after }
    }

    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.read().unwrap_or_else(|p| p.into_inner()).len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn remove(&self, key: &K) -> bool {
        let shard = &self.shards[self.shard_index(key)];
        shard.write().unwrap_or_else(|p| p.into_inner()).remove(key).is_some()
    }

    fn shard_index(&self, key: &K) -> usize {
        (self.hash_builder.hash_one(key) as usize) % self.shards.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use crate::quota::{Quota, RefillRate};
    use std::num::NonZeroUsize;
    use std::time::Duration;

    fn nz(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    fn quota(capacity: u32, per_sec: u32) -> Quota {
        Quota::new(nz(capacity), RefillRate::per_second(nz(per_sec)))
    }

    #[test]
    fn a_brand_new_key_starts_full_regardless_of_other_keys() {
        let cfg = KeyedLimiterConfig::new(quota(5, 1));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        // Drain "a" completely.
        limiter.check_n(&"a", nz(5));
        // "b" has never been seen: it must be judged only against capacity.
        assert_eq!(limiter.check_n(&"b", nz(5)), Decision::Allowed { remaining: 0 });
    }

    #[test]
    fn each_key_has_an_independent_bucket() {
        let cfg = KeyedLimiterConfig::new(quota(5, 1));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        assert_eq!(limiter.check(&"a"), Decision::Allowed { remaining: 4 });
        assert_eq!(limiter.check(&"b"), Decision::Allowed { remaining: 4 });
        assert_eq!(limiter.check(&"a"), Decision::Allowed { remaining: 3 });
    }

    #[test]
    fn over_capacity_is_unsatisfiable() {
        let cfg = KeyedLimiterConfig::new(quota(5, 1));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        assert_eq!(
            limiter.check_n(&"a", nz(6)),
            Decision::Unsatisfiable { capacity: 5, requested: 6 }
        );
    }

    #[test]
    fn len_and_is_empty_track_tracked_keys() {
        let cfg = KeyedLimiterConfig::new(quota(5, 1));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        assert!(limiter.is_empty());
        limiter.check(&"a");
        limiter.check(&"b");
        assert_eq!(limiter.len(), 2);
        assert!(!limiter.is_empty());
    }

    #[test]
    fn remove_forgets_a_key_so_it_starts_fresh_again() {
        let cfg = KeyedLimiterConfig::new(quota(5, 1));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        limiter.check_n(&"a", nz(5)); // drain
        assert!(limiter.remove(&"a"));
        assert_eq!(limiter.check_n(&"a", nz(5)), Decision::Allowed { remaining: 0 });
    }

    #[test]
    fn remove_of_unknown_key_reports_false() {
        let cfg = KeyedLimiterConfig::new(quota(5, 1));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        assert!(!limiter.remove(&"nope"));
    }

    #[test]
    fn sweep_is_a_no_op_without_idle_eviction_configured() {
        let cfg = KeyedLimiterConfig::new(quota(5, 1));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        limiter.check(&"a");
        let report = limiter.sweep();
        assert_eq!(report.keys_before, 1);
        assert_eq!(report.keys_removed, 0);
        assert_eq!(report.keys_after, 1);
    }

    #[test]
    fn sweep_removes_keys_idle_past_the_threshold() {
        let clock = FakeClock::new();
        let mut cfg = KeyedLimiterConfig::new(quota(5, 1));
        cfg.idle_eviction = Some(Duration::from_secs(10));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, clock.clone());
        limiter.check(&"a");
        clock.advance(Duration::from_secs(20));
        limiter.check(&"b"); // touched recently, should survive
        let report = limiter.sweep();
        assert_eq!(report.keys_before, 2);
        assert_eq!(report.keys_removed, 1);
        assert_eq!(report.keys_after, 1);
        assert!(!limiter.is_empty());
        // "a" is gone, so it starts fresh (full) again.
        assert_eq!(limiter.check_n(&"a", nz(5)), Decision::Allowed { remaining: 0 });
    }

    #[test]
    fn an_evicted_key_never_carries_debt_or_bonus() {
        let clock = FakeClock::new();
        let mut cfg = KeyedLimiterConfig::new(quota(5, 1));
        cfg.idle_eviction = Some(Duration::from_secs(10));
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, clock.clone());
        limiter.check_n(&"a", nz(3)); // partially drain, but not mid-request
        clock.advance(Duration::from_secs(20));
        limiter.sweep();
        // Same as any unseen key: full capacity, not partial and not extra.
        assert_eq!(limiter.check_n(&"a", nz(5)), Decision::Allowed { remaining: 0 });
    }

    #[test]
    fn reject_new_keys_denies_rather_than_evicting() {
        let cfg = KeyedLimiterConfig {
            max_keys: Some(NonZeroUsize::new(1).unwrap()),
            eviction_policy: EvictionPolicy::RejectNewKeys,
            shard_count: NonZeroUsize::new(1).unwrap(),
            ..KeyedLimiterConfig::new(quota(5, 1))
        };
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        limiter.check(&"a");
        assert_eq!(limiter.check(&"b"), Decision::Denied { retry_after: Duration::ZERO });
        // "a" must still be tracked; "b" was refused, not admitted.
        assert_eq!(limiter.len(), 1);
    }

    #[test]
    fn max_keys_is_never_exceeded_under_an_adversarial_key_stream() {
        let cfg = KeyedLimiterConfig {
            max_keys: Some(NonZeroUsize::new(3).unwrap()),
            eviction_policy: EvictionPolicy::ApproximateLru,
            shard_count: NonZeroUsize::new(1).unwrap(),
            ..KeyedLimiterConfig::new(quota(5, 1))
        };
        let limiter: KeyedRateLimiter<u64, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        for key in 0..1000u64 {
            limiter.check(&key);
            assert!(limiter.len() <= 3, "len {} exceeded max_keys=3 at key {key}", limiter.len());
        }
        assert_eq!(limiter.len(), 3);
    }

    #[test]
    fn approximate_lru_evicts_the_least_recently_touched_key() {
        let cfg = KeyedLimiterConfig {
            max_keys: Some(NonZeroUsize::new(2).unwrap()),
            eviction_policy: EvictionPolicy::ApproximateLru,
            shard_count: NonZeroUsize::new(1).unwrap(),
            ..KeyedLimiterConfig::new(quota(5, 1))
        };
        let limiter: KeyedRateLimiter<&str, _> = KeyedRateLimiter::new(cfg, FakeClock::new());
        limiter.check(&"a");
        limiter.check(&"b");
        limiter.check(&"a"); // touch "a" again; "b" is now the least-recently-touched
        limiter.check(&"c"); // should evict "b", not "a"
        assert_eq!(limiter.len(), 2);
        assert!(!limiter.remove(&"b")); // "b" should already be gone
        assert!(limiter.remove(&"a"));
        // put "a" back for a clean assertion of "c" presence too
        limiter.check(&"a");
        assert!(limiter.remove(&"c"));
    }
}
