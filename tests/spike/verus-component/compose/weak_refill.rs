// Feasibility probe: can the composition obligations be discharged from
// contracts ALONE -- no function bodies, no vstd, no trusted wrappers?
//
// The existing spike (proof/bucket.rs) proves the bucket's implementation.
// This asks the different question the brief asks: taking each function's
// contract as a premise, do they ENTAIL the component invariant?
use vstd::prelude::*;

verus! {

/// The abstract state: exactly the observers the theorem mentions. Values
/// are mathematical integers, with the range facts of their real Rust types
/// carried as explicit premises -- NOT unbounded arithmetic smuggled in.
pub struct S { pub available: int, pub capacity: int }

/// Range facts derived from the declared Rust types (both u32).
pub open spec fn typed(s: S) -> bool {
    0 <= s.available <= 4294967295 && 0 <= s.capacity <= 4294967295
}

/// The component invariant, from `holds:`.
pub open spec fn inv(s: S) -> bool { s.available <= s.capacity }

/// Constructor contract: `new(capacity)` ensures ...
pub open spec fn new_post(cap: int, s: S) -> bool {
    s.available == cap && s.capacity == cap
}

/// `try_take` two-state contract, exactly as the Rust promise states it.
pub open spec fn try_take_post(pre: S, tokens: int, post: S, ok: bool) -> bool {
    (ok == (pre.available >= tokens))
    && (ok ==> post.available == pre.available - tokens)
    && (!ok ==> post.available == pre.available)
    && (post.capacity == pre.capacity)
}

/// `refill` two-state contract.
pub open spec fn refill_post(pre: S, tokens: int, post: S) -> bool {
    (post.available >= pre.available)
    && (post.capacity == pre.capacity)
}

// ---- The obligations. No bodies anywhere; premises only. ----

/// Initialization: every state the constructor can produce satisfies I.
proof fn ob_init(cap: int, s: S)
    requires 0 <= cap <= 4294967295, typed(s), new_post(cap, s)
    ensures inv(s)
{ }

/// Preservation for try_take.
proof fn ob_preserve_try_take(pre: S, tokens: int, post: S, ok: bool)
    requires inv(pre), typed(pre), typed(post), 0 <= tokens <= 4294967295,
             try_take_post(pre, tokens, post, ok)
    ensures inv(post)
{ }

/// Preservation for refill.
proof fn ob_preserve_refill(pre: S, tokens: int, post: S)
    requires inv(pre), typed(pre), typed(post), 0 <= tokens <= 4294967295,
             refill_post(pre, tokens, post)
    ensures inv(post)
{ }

fn main() { }
}
