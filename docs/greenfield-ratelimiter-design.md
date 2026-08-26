# Flowgate: a token-bucket rate limiter

## 1. Purpose

Flowgate is a library for deciding, on every incoming request, whether to let it through
right now, and if not, how long the caller should wait before trying again. It is meant to
sit inside a service — behind an HTTP handler, in front of an expensive downstream call, or
guarding a per-user action — as the thing that answers "allowed or not" in a few dozen
nanoseconds without doing any I/O. It supports a single global limit as well as an
independent limit per key (per user ID, per API token, per source IP), and it separates the
question "what time is it" from the rest of the logic so that limiter behavior can be tested
without sleeping in a test suite.

It does not do anything about *what happens* when a request is denied (queuing, shedding,
responding with a particular HTTP status) — that policy belongs to the caller. Flowgate only
answers the yes/no/how-long question, as cheaply and correctly as it can.

## 2. Public API

### 2.1 Time

Every limiter in this crate is generic over a clock, so that tests can control time exactly
instead of racing the real one.

```rust
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
```

For tests, a manually-driven clock:

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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
```

### 2.2 Quota and configuration

```rust
use std::num::{NonZeroU32, NonZeroUsize};

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

    fn tokens_per_nanosecond(&self) -> f64 {
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
```

Configuration for the per-key limiter, including how it bounds its own memory:

```rust
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
```

### 2.3 Decisions

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    /// The request is admitted now. `remaining` is how many tokens were
    /// left in the bucket immediately after this request was debited.
    Allowed { remaining: u32 },
    /// Not admitted right now. Waiting at least `retry_after` and then
    /// retrying the same request (assuming nobody else spends tokens from
    /// this bucket in the meantime) would succeed.
    Denied { retry_after: Duration },
    /// The request asked for more tokens than the bucket can ever hold at
    /// full capacity. No amount of waiting makes this request satisfiable;
    /// it is a caller bug (or a cost model mismatch), not a rate-limit
    /// event, and callers should not retry it unchanged.
    Unsatisfiable { capacity: u32, requested: u32 },
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allowed { .. })
    }
}
```

### 2.4 Single bucket

The unkeyed limiter — one bucket, one quota — used directly for a global limit, and as the
model the per-key limiter replicates per key.

```rust
use std::sync::Mutex;

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
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
```

### 2.5 Per-key limiter

```rust
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::AtomicU64;
use std::sync::RwLock;

pub struct KeyedRateLimiter<K, C = MonotonicClock, S = RandomState>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    C: Clock,
    S: BuildHasher + Clone,
{
    config: KeyedLimiterConfig,
    clock: C,
    hash_builder: S,
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

impl<K, C, S> KeyedRateLimiter<K, C, S>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    C: Clock,
    S: BuildHasher + Clone + Default,
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
                        return Decision::Denied { retry_after: Duration::ZERO };
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
        let mut hasher = self.hash_builder.build_hasher();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.shards.len()
    }
}
```

### 2.6 A shared trait for callers who don't care which kind they have

```rust
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
```

## 3. Internal types

These are not exported; they exist to hold the state the public API above operates on.

```rust
/// The state of one bucket: how many tokens (possibly fractional, since
/// refill happens continuously rather than in discrete steps) it currently
/// holds, and when that count was last brought up to date.
struct BucketState<I> {
    tokens: f64,
    updated_at: I,
}

/// The per-key equivalent of `BucketState`, with an extra field purely for
/// approximate-LRU bookkeeping: a monotonically increasing counter stamped
/// on every touch (creation or check), so eviction can find "the entry
/// nobody has touched in the longest time" within a shard by a linear scan
/// for the minimum. That scan is O(shard size); it runs only when the shard
/// is at its configured key cap, not on every request.
struct Entry<I> {
    tokens: f64,
    updated_at: I,
    sequence: u64,
}

/// The refill-and-debit arithmetic shared by `TokenBucket` and
/// `KeyedRateLimiter`, so the two public types can't drift into checking
/// slightly different rules. Mutates `tokens` and `updated_at` in place and
/// returns the resulting decision. Assumes `requested <= quota.capacity()`
/// has already been checked by the caller — this function does not repeat
/// that check, since by this point it always holds.
fn refill_and_debit<C: Clock>(
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
```

## 4. Invariants

1. **A bucket's token count never exceeds its configured capacity, no matter how much time
   passes between checks.** A service that checks a key once and doesn't see it again for a
   month must not come back to find it can suddenly absorb a burst bigger than its capacity.
   Constrains: `refill_and_debit`, `TokenBucket::check_n`, `KeyedRateLimiter::check_n`.

2. **A bucket's token count never goes negative, and never gets debited unless the full
   request is satisfied.** There is no partial admission. Constrains: `refill_and_debit`.

3. **The internal "last updated" timestamp only ever moves forward, even when the
   underlying clock briefly reports a time earlier than the last one observed.** A clock
   hiccup must cost zero elapsed time, not produce a negative one, and must not let the
   bucket "catch up" on a gap that never happened once the clock resumes normal progress.
   Constrains: `refill_and_debit`'s timestamp update, and every `Clock::duration_since`
   implementation.

4. **A request for more tokens than the bucket's own capacity is never granted, at any
   delay, and is never reported as a plain `Denied` with some `retry_after` — it is reported
   as `Unsatisfiable`.** Telling such a caller "try again in 4 seconds" would be a lie; there
   is no wait after which it succeeds. Constrains: `TokenBucket::check_n`,
   `KeyedRateLimiter::check_n`, `TokenBucket::time_until_ready`.

5. **A `retry_after` returned alongside `Denied` is long enough that retrying after exactly
   that wait (with no other consumption in between) succeeds — and is not so much longer
   than necessary that it starves a caller who could have gone sooner.** This is the
   invariant I'd trust least from a read-through alone, because it depends on floating-point
   division and a ceiling operation lining up exactly with the discrete token math used by
   the admit path. Constrains: `refill_and_debit`'s `Denied` arm.

6. **A key that has never been checked before is judged only against capacity, never
   against any other key's history or any assumed prior consumption.** The very first
   request for a brand-new user or IP must not be spuriously denied because of unrelated
   traffic. Constrains: `KeyedRateLimiter::check_n`'s insertion path.

7. **Concurrent calls to `check`/`check_n` — on the same key, on different keys, from any
   number of threads — never collectively admit more than the bucket math allows for the
   elapsed time.** All reads and mutations of one bucket's state happen under that bucket's
   own lock; two threads racing on the same key must serialize, not each see a stale token
   count and both proceed. This is the second invariant I'd trust least from reading the
   code alone — the fast-path/slow-path double-check in `check_n` exists specifically to
   close a window where two threads could otherwise both decide a key is unseen and both
   try to insert it, but proving no other such window exists is a matter of careful
   reasoning about lock scope, not something visibly obvious from the shape of the function.
   Constrains: `TokenBucket::check_n`'s locking, `KeyedRateLimiter::check_n`'s fast/slow path.

8. **Evicting an idle key (via `sweep`, or via `ApproximateLru` making room for a new one)
   is always equivalent to that key never having been seen — never a way to either grant it
   extra tokens or hand it a debt it didn't earn.** Concretely: eviction never happens to a
   key with an outstanding partially-processed request, and the very next check for an
   evicted key starts it at full capacity, same as any other unseen key. Constrains:
   `KeyedRateLimiter::sweep`, the `ApproximateLru` branch of `check_n`.

9. **With `max_keys` set, the number of distinct keys tracked never exceeds it, regardless
   of how many distinct keys are ever requested — including an adversarial stream designed
   to exhaust memory by using a new key on every request.** Constrains: the `max_keys`
   branch of `KeyedRateLimiter::check_n`.

10. **Configuration that cannot describe a real rate is rejected at construction time, not
    accepted and left to misbehave later.** A zero-duration refill interval is the concrete
    case (it would describe an infinite rate); it is a `ConfigError`, not a runtime panic or
    a silent division producing infinity or NaN. Constrains: `RefillRate::new`.

11. **The `remaining` count in an `Allowed` decision reflects the bucket's real state at
    that instant** — it is what an immediate follow-up `check_n(1)` on the same key would
    also see, not a cached or optimistic estimate computed before the debit. Constrains:
    `refill_and_debit`'s `Allowed` arm.

## 5. What I'm least sure of

The two invariants above that I've marked explicitly (5 and 7) are the honest hard parts.
Getting the discrete "how many whole tokens are actually available" question to agree
exactly with the continuous floating-point accrual math, across a refill rate that doesn't
divide evenly into nanoseconds, over a bucket that's been running for a long time (where
small rounding errors in repeated `f64` addition could in principle drift), is the kind of
thing that looks right in every small example and still hides a one-nanosecond-off boundary
case. And while the locking discipline in `check_n` is deliberately simple — one mutex per
bucket, held for the whole read-modify-write — convincing myself that the unseen-key
double-check closes every race between two threads discovering the same new key at once,
rather than just the most obvious one, is exactly the kind of thing I'd want a second pair
of eyes and a stress test on before shipping, not just a careful re-read of the function.
