// The corrected shadow of `tests/fixtures/boundedcache`, replacing the first
// attempt in git history.
//
// Two things were wrong with that attempt, both found by adversarial review:
// its fields were `pub` where the fixture's are private -- so it threw away
// the boundary obligation it then declared unreachable -- and it wrote the
// invariant as per-method pre/postconditions, which only ever answers about
// the methods you list.
//
// This version uses `#[verifier::type_invariant]`. It rejects an
// uncontracted mutator and a free function in the same module that reaches
// into the struct, and it refuses a struct whose fields are public to the
// crate. All four obligations are the verifier's.
//
// The cost is below the impl: vstd's `Vec::push`/`remove` are not marked
// `no_unwind`, and a type invariant may not be left broken across a call
// that can unwind, so each needs a trusted wrapper. `push` genuinely can
// unwind on capacity overflow, so these are a real trust surface and not a
// formality.
use vstd::prelude::*;
verus! {
pub struct Cache { capacity: usize, entries: Vec<(u32, u32)> }
impl Cache {
    #[verifier::type_invariant]
    pub closed spec fn inv(self) -> bool { self.entries.len() <= self.capacity }
    pub closed spec fn spec_len(self) -> int { self.entries.len() as int }
    pub closed spec fn spec_capacity(self) -> int { self.capacity as int }
    pub fn with_capacity(capacity: usize) -> (result: Cache) { Cache { capacity, entries: Vec::new() } }
    pub fn len(&self) -> (n: usize) ensures n == self.spec_len() { self.entries.len() }
    pub fn capacity(&self) -> (n: usize) ensures n == self.spec_capacity() { self.capacity }
    fn find(&self, key: u32) -> (found: Option<usize>)
        ensures match found { Some(i) => i < self.entries.len(), None => true }
    {
        let mut i: usize = 0;
        while i < self.entries.len()
            invariant i <= self.entries.len()
            decreases self.entries.len() - i
        {
            if self.entries[i].0 == key { return Some(i); }
            i = i + 1;
        }
        None
    }
    pub fn put(&mut self, key: u32, value: u32) {
        proof { use_type_invariant(&*self); }
        if self.capacity == 0 { return; }
        match self.find(key) {
            Some(pos) => { remove_nu(&mut self.entries, pos); push_nu(&mut self.entries, (key, value)); }
            None => {
                if self.entries.len() >= self.capacity { remove_nu(&mut self.entries, 0); }
                push_nu(&mut self.entries, (key, value));
            }
        }
    }
    pub fn get(&mut self, key: u32) -> (r: Option<u32>) {
        proof { use_type_invariant(&*self); }
        match self.find(key) {
            Some(pos) => { let entry = remove_nu(&mut self.entries, pos); push_nu(&mut self.entries, entry); Some(entry.1) }
            None => None,
        }
    }
    // Uncontracted public mutator: the spike's coverage hole.
}
// A free function in the defining module: not a method, invisible to a receiver scan.

#[verifier::external_body]
fn push_nu(v: &mut Vec<(u32, u32)>, x: (u32, u32))
    ensures final(v)@ == old(v)@.push(x)
    no_unwind
{ v.push(x) }
#[verifier::external_body]
fn remove_nu(v: &mut Vec<(u32, u32)>, i: usize) -> (e: (u32, u32))
    requires i < old(v).len()
    ensures e == old(v)[i as int], final(v)@ == old(v)@.remove(i as int)
    no_unwind
{ v.remove(i) }
#[verifier::external_body]
fn pop_nu(v: &mut Vec<(u32, u32)>)
    requires old(v).len() > 0
    ensures final(v)@ == old(v)@.drop_last()
    no_unwind
{ v.pop(); }
fn main() {}
}
