//! The decisive fixture for receiver construction
//! (docs/review-self-construction.md's "fourth option", task 2026-08-27):
//! an invariant that holds on every single call from a freshly built
//! receiver, and breaks only after a *second* call -- the exact shape the
//! review proved constructor-only cannot reach ("a freshly built token
//! bucket starts full ... the deny branch is unreachable by construction").
//!
//! `Meter` always starts with 10 units. `spend`'s own precondition
//! (`amount <= 10`) makes the very first call on a fresh `Meter` safe by
//! construction: `remaining` starts at 10, so `10 - amount` can never go
//! negative. But `spend` mutates `remaining` through it (interior
//! mutability, `&self` -- the shape Ply's receiver construction builds a
//! sequence for), so a *second* call sees whatever `remaining` the first
//! call left behind, which can be smaller than the second call's own
//! `amount` -- and `remaining - amount` underflows, which panics in a debug
//! build. One call can never trigger this; two (or more) reliably can.

pub struct Meter {
    remaining: std::cell::Cell<u32>,
}

impl Meter {
    /// Fixed starting capacity, no parameters -- deliberately so the only
    /// thing that can vary the receiver `spend`'s own sequence sees is the
    /// sequence itself, not the constructor's args.
    pub fn new() -> Self {
        Meter {
            remaining: std::cell::Cell::new(10),
        }
    }

    /// Always safe on a *fresh* `Meter`: `remaining == 10` and `amount <=
    /// 10`, so `remaining - amount` never underflows. Unsafe on a `Meter`
    /// that has already been spent from: `remaining` may be smaller than
    /// `amount`, even though `amount` alone still satisfies `requires`.
    #[ply::requires(amount <= 10)]
    #[ply::ensures(|result| *result <= 10)]
    pub fn spend(&self, amount: u32) -> u32 {
        let cur = self.remaining.get();
        let next = cur - amount;
        self.remaining.set(next);
        next
    }
}
