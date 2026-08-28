//! Invariant 7: concurrent calls to `check`/`check_n` never collectively
//! admit more than the bucket math allows. Exercised here with real OS
//! threads racing on the same bucket and the same key, since this is
//! exactly the kind of bug that a single-threaded test can't see.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use flowgate::{KeyedLimiterConfig, KeyedRateLimiter, MonotonicClock, Quota, RefillRate, TokenBucket};

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

#[test]
fn a_shared_token_bucket_never_admits_more_than_its_capacity_under_contention() {
    const CAPACITY: u32 = 100;
    const THREADS: usize = 16;
    const ATTEMPTS_PER_THREAD: usize = 200;

    // Use the real clock but with an effectively infinite refill interval,
    // so the only tokens available are the ones the bucket started with --
    // any admitted count above CAPACITY would prove a race, not a refill.
    let quota = Quota::new(CAPACITY.try_into().unwrap(), RefillRate::new(nz(1), std::time::Duration::from_secs(3600)).unwrap());
    let bucket = Arc::new(TokenBucket::new(quota, MonotonicClock));
    let admitted = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let bucket = Arc::clone(&bucket);
            let admitted = Arc::clone(&admitted);
            std::thread::spawn(move || {
                for _ in 0..ATTEMPTS_PER_THREAD {
                    if bucket.check().is_allowed() {
                        admitted.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let total = admitted.load(Ordering::SeqCst);
    assert!(
        total <= CAPACITY as usize,
        "admitted {total} requests against a bucket of capacity {CAPACITY}"
    );
}

#[test]
fn concurrent_first_touches_of_the_same_new_key_still_admit_at_most_capacity() {
    const CAPACITY: u32 = 20;
    const THREADS: usize = 32;

    let quota = Quota::new(CAPACITY.try_into().unwrap(), RefillRate::new(nz(1), std::time::Duration::from_secs(3600)).unwrap());
    let limiter: Arc<KeyedRateLimiter<&'static str, MonotonicClock>> =
        Arc::new(KeyedRateLimiter::new(KeyedLimiterConfig::new(quota), MonotonicClock));
    let admitted = Arc::new(AtomicUsize::new(0));

    // All threads race to be the first to touch the same never-before-seen
    // key -- this is exactly the fast-path/slow-path race the design
    // document calls out as the one it trusts least.
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let limiter = Arc::clone(&limiter);
            let admitted = Arc::clone(&admitted);
            std::thread::spawn(move || {
                if limiter.check(&"contested-key").is_allowed() {
                    admitted.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(limiter.len(), 1, "exactly one bucket should exist for the contested key");
    let total = admitted.load(Ordering::SeqCst);
    assert!(
        total <= CAPACITY as usize,
        "admitted {total} requests against a bucket of capacity {CAPACITY}"
    );
}
