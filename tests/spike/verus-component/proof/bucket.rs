// The round-3 token bucket, shadowed twice over: once as a type invariant
// (`available <= capacity`, the only thing `state:` can say today) and once
// as two-state contracts on the mutating methods (what `try_take` and
// `refill` actually promise).
//
// This is the measurement that decides the fork in
// `docs/component-proof-design.md`. Of that scenario's three pre-registered
// bugs, exactly one violates the invariant; the other two are transition
// bugs, and they are the shape the A/B rounds kept measuring as unreachable.
use vstd::prelude::*;

verus! {

pub struct TokenBucket {
    capacity: u32,
    available: u32,
}

impl TokenBucket {
    /// Obligations 1-4, the `holds:` clause. Fields are private, as the real
    /// fixture's are: a type invariant refuses a struct whose fields are
    /// public to the crate, which is the boundary obligation enforced.
    #[verifier::type_invariant]
    pub closed spec fn inv(self) -> bool {
        self.available <= self.capacity
    }

    pub closed spec fn spec_available(self) -> int { self.available as int }
    pub closed spec fn spec_capacity(self) -> int { self.capacity as int }

    pub fn new(capacity: u32) -> (r: TokenBucket)
        ensures
            r.spec_available() == capacity,
            r.spec_capacity() == capacity,
    {
        TokenBucket { capacity, available: capacity }
    }

    pub fn available(&self) -> (n: u32)
        ensures n == self.spec_available()
    { self.available }

    pub fn capacity(&self) -> (n: u32)
        ensures n == self.spec_capacity()
    { self.capacity }

    /// A transition property, which no invariant over a single state can
    /// say: a successful take removes exactly `tokens`, a failed one changes
    /// nothing at all, and the capacity never moves.
    pub fn try_take(&mut self, tokens: u32) -> (ok: bool)
        ensures
            ok == (old(self).spec_available() >= tokens),
            ok ==> final(self).spec_available() == old(self).spec_available() - tokens,
            !ok ==> final(self).spec_available() == old(self).spec_available(),
            final(self).spec_capacity() == old(self).spec_capacity(),
    {
        proof { use_type_invariant(&*self); }
        if self.available >= tokens {
            self.available = self.available - tokens;
            true
        } else {
            false
        }
    }

    /// The other transition: a refill adds exactly what was asked for, and
    /// stops at the top. `refill(0)` changing nothing is a consequence of
    /// this, not a separate clause.
    pub fn refill(&mut self, tokens: u32)
        ensures
            final(self).spec_available() == if old(self).spec_available() + tokens > old(self).spec_capacity() {
                old(self).spec_capacity()
            } else {
                old(self).spec_available() + tokens
            },
            final(self).spec_capacity() == old(self).spec_capacity(),
    {
        proof { use_type_invariant(&*self); }
        let room = self.capacity - self.available;
        if tokens >= room {
            self.available = self.capacity;
        } else {
            self.available = self.available + tokens;
        }
    }
}

fn main() {}

}
