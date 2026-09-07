//! A rule about a structure is only checked as far as the generated call
//! sequences can drive that structure.
//!
//! `state: { of: Cache, holds: [state.len() <= state.capacity()] }` is the
//! plainest invariant there is, and until 2026-09-07 an off-by-one that let
//! this cache hold one entry too many came back clean over 256 cases. The
//! sequences were capped at three calls while the capacity was drawn from
//! 0..=16, so almost every generated cache was far larger than the sequence
//! could ever fill. Replaying Ply's own generated strategy over twelve seeds
//! measured **2 of 3,072 cases** able to reach the bug at all.
//!
//! Found in an A/B round by an agent that declared exactly the right rule,
//! broke its own eviction test to see whether Ply would notice, and reported
//! that it did not.

/// A bounded cache that drops its oldest entry to make room.
pub struct Cache {
    capacity: usize,
    entries: Vec<(u32, u32)>,
}

impl Cache {
    pub fn with_capacity(capacity: usize) -> Cache {
        Cache {
            capacity,
            entries: Vec::new(),
        }
    }

    pub fn put(&mut self, key: u32, value: u32) {
        if self.capacity == 0 {
            return;
        }
        if let Some(pos) = self.entries.iter().position(|&(k, _)| k == key) {
            self.entries.remove(pos);
            self.entries.push((key, value));
            return;
        }
        // The fullness test. `>=` is right: the push below adds one, so a
        // cache already holding `capacity` entries has to drop one first.
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
        }
        self.entries.push((key, value));
    }

    pub fn get(&mut self, key: u32) -> Option<u32> {
        let pos = self.entries.iter().position(|&(k, _)| k == key)?;
        let (_, value) = self.entries.remove(pos);
        self.entries.push((key, value));
        Some(value)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}
