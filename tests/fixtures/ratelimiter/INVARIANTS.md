# Flowgate invariants

Copied verbatim (numbering preserved) from §4 of `docs/greenfield-ratelimiter-design.md`,
the source design document this fixture (`tests/fixtures/ratelimiter/`) implements. Each
entry lists the function(s) it constrains, using this crate's actual paths.

1. **A bucket's token count never exceeds its configured capacity, no matter how much time
   passes between checks.** A service that checks a key once and doesn't see it again for a
   month must not come back to find it can suddenly absorb a burst bigger than its capacity.
   Constrains: `internal::refill_and_debit`, `TokenBucket::check_n`,
   `KeyedRateLimiter::check_n`.

2. **A bucket's token count never goes negative, and never gets debited unless the full
   request is satisfied.** There is no partial admission. Constrains:
   `internal::refill_and_debit`.

3. **The internal "last updated" timestamp only ever moves forward, even when the
   underlying clock briefly reports a time earlier than the last one observed.** A clock
   hiccup must cost zero elapsed time, not produce a negative one, and must not let the
   bucket "catch up" on a gap that never happened once the clock resumes normal progress.
   Constrains: `internal::refill_and_debit`'s timestamp update, and every `Clock::duration_since`
   implementation (`MonotonicClock`, `WallClock`, `FakeClock`).

4. **A request for more tokens than the bucket's own capacity is never granted, at any
   delay, and is never reported as a plain `Denied` with some `retry_after` — it is reported
   as `Unsatisfiable`.** Telling such a caller "try again in 4 seconds" would be a lie; there
   is no wait after which it succeeds. Constrains: `TokenBucket::check_n`,
   `KeyedRateLimiter::check_n`, `TokenBucket::time_until_ready`.

5. **A `retry_after` returned alongside `Denied` is long enough that retrying after exactly
   that wait (with no other consumption in between) succeeds — and is not so much longer
   than necessary that it starves a caller who could have gone sooner.** This is the
   invariant the design document's author says they'd trust least from a read-through
   alone, because it depends on floating-point division and a ceiling operation lining up
   exactly with the discrete token math used by the admit path. Constrains:
   `internal::refill_and_debit`'s `Denied` arm.

6. **A key that has never been checked before is judged only against capacity, never
   against any other key's history or any assumed prior consumption.** The very first
   request for a brand-new user or IP must not be spuriously denied because of unrelated
   traffic. Constrains: `KeyedRateLimiter::check_n`'s insertion path.

7. **Concurrent calls to `check`/`check_n` — on the same key, on different keys, from any
   number of threads — never collectively admit more than the bucket math allows for the
   elapsed time.** All reads and mutations of one bucket's state happen under that bucket's
   own lock; two threads racing on the same key must serialize, not each see a stale token
   count and both proceed. This is the second invariant the design document's author says
   they'd trust least from reading the code alone — the fast-path/slow-path double-check in
   `check_n` exists specifically to close a window where two threads could otherwise both
   decide a key is unseen and both try to insert it, but proving no other such window exists
   is a matter of careful reasoning about lock scope, not something visibly obvious from the
   shape of the function. Constrains: `TokenBucket::check_n`'s locking,
   `KeyedRateLimiter::check_n`'s fast/slow path.

8. **Evicting an idle key (via `sweep`, or via `ApproximateLru` making room for a new one)
   is always equivalent to that key never having been seen — never a way to either grant it
   extra tokens or hand it a debt it didn't earn.** Concretely: eviction never happens to a
   key with an outstanding partially-processed request, and the very next check for an
   evicted key starts it at full capacity, same as any other unseen key. Constrains:
   `KeyedRateLimiter::sweep`, the `ApproximateLru` branch of `KeyedRateLimiter::check_n`.

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
    `internal::refill_and_debit`'s `Allowed` arm.
