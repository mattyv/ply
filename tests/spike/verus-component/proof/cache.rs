// A Verus shadow of `tests/fixtures/boundedcache`, written to answer one
// question: can a deductive verifier discharge a `state:` invariant's init
// and preservation obligations for a shape a user would actually write?
//
// The kernel spike proved a pure recursive tree. This is a `Vec` of pairs
// mutated in place, which is a different proposition. The invariant is the
// fixture's own: the cache never holds more entries than its capacity.
use vstd::prelude::*;

verus! {

pub struct Cache {
    pub capacity: usize,
    pub entries: Vec<(u32, u32)>,
}

impl Cache {
    /// The `holds:` clause, as a specification.
    pub open spec fn inv(self) -> bool {
        self.entries.len() <= self.capacity
    }

    /// Obligation 1, init: every value the constructor can build satisfies
    /// the invariant.
    pub fn with_capacity(capacity: usize) -> (result: Cache)
        ensures result.inv()
    {
        Cache { capacity, entries: Vec::new() }
    }

    pub fn len(&self) -> (n: usize)
        ensures n == self.entries.len()
    {
        self.entries.len()
    }

    pub fn capacity(&self) -> (n: usize)
        ensures n == self.capacity
    {
        self.capacity
    }

    /// The fixture writes this as `entries.iter().position(..)`. A faithful
    /// shadow spells the search out as a loop, because that is what the
    /// verifier can reason about -- and the cost of that translation is part
    /// of what this spike measures.
    fn find(&self, key: u32) -> (found: Option<usize>)
        ensures match found {
            Some(i) => i < self.entries.len(),
            None => true,
        }
    {
        let mut i: usize = 0;
        while i < self.entries.len()
            invariant i <= self.entries.len()
            decreases self.entries.len() - i
        {
            if self.entries[i].0 == key {
                return Some(i);
            }
            i = i + 1;
        }
        None
    }

    /// Obligation 2, preservation, for the operation that can break it.
    pub fn put(&mut self, key: u32, value: u32)
        requires old(self).inv()
        ensures final(self).inv()
    {
        if self.capacity == 0 {
            return;
        }
        match self.find(key) {
            Some(pos) => {
                self.entries.remove(pos);
                self.entries.push((key, value));
            }
            None => {
                if self.entries.len() >= self.capacity {
                    self.entries.remove(0);
                }
                self.entries.push((key, value));
            }
        }
    }

    /// Preservation for the other mutator. A `get` moves the entry it finds
    /// to the back, so it changes the value even though it reads.
    pub fn get(&mut self, key: u32) -> (r: Option<u32>)
        requires old(self).inv()
        ensures final(self).inv()
    {
        match self.find(key) {
            Some(pos) => {
                let entry = self.entries.remove(pos);
                self.entries.push(entry);
                Some(entry.1)
            }
            None => None,
        }
    }
}

fn main() {}

}
