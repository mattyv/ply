//! Five ordinary shapes that used to make the shared harness fail to
//! compile, turning every function in the crate into a tool error with raw
//! compiler output (docs/review-structs-enums.md's "Also fix" list,
//! 2026-08-28). Each must now be refused by name before generation --
//! except the second, where a working fallback exists and refusing would
//! throw it away -- leaving `normal_fn` (and every other function) checked
//! normally, its own real bug still found.

/// Shape 1: a non-public type with public fields and no usable constructor.
/// Direct field construction would otherwise apply (every field is
/// public), but the type itself cannot even be named from the fuzz harness
/// Ply generates outside this module.
pub(crate) struct Hidden {
    pub x: u32,
}

#[ply::ensures(|result| *result)]
pub fn uses_hidden(h: Hidden) -> bool {
    h.x == h.x
}

/// Shape 2: a public type with public fields *and* a private constructor.
/// The private constructor is not a route Ply can use, but direct field
/// construction is -- and was available all along, since every field is
/// already public. This one must actually be CHECKED, not refused: refusing
/// it would throw away a working fallback.
pub struct WithPrivateCtor {
    pub x: u32,
}

impl WithPrivateCtor {
    fn new(x: u32) -> Self {
        WithPrivateCtor { x }
    }
}

#[ply::ensures(|result| *result)]
pub fn uses_privctor(w: WithPrivateCtor) -> bool {
    w.x == w.x
}

/// Shape 3: a struct with 13 public fields and no constructor -- one more
/// than direct construction's generated strategy (a tuple, one slot per
/// field) can build.
pub struct Big13 {
    pub f0: u32,
    pub f1: u32,
    pub f2: u32,
    pub f3: u32,
    pub f4: u32,
    pub f5: u32,
    pub f6: u32,
    pub f7: u32,
    pub f8: u32,
    pub f9: u32,
    pub f10: u32,
    pub f11: u32,
    pub f12: u32,
}

#[ply::ensures(|result| *result)]
pub fn uses_big13(b: Big13) -> bool {
    // Deliberately false only when the *thirteenth* field is large. Before
    // 2026-09-04 this whole shape was refused, on the grounds that a
    // struct's generated recipe is one flat tuple and the sampling
    // library's trait for those stops at twelve. That was a fact about
    // Ply's own folding, not about the struct: nesting the tuple lifts it.
    // So this promise is the proof the thirteenth leaf is really drawn --
    // if it were quietly left at its default, the check would come back
    // green and mean nothing.
    //
    // The threshold is small on purpose, and it is the whole trick: a
    // default `u32` is 0, so if the thirteenth leaf were quietly left at
    // its default this promise would HOLD and the check would pass. It
    // fails only when that field is really being drawn. A large threshold
    // was tried first and was the wrong instrument -- it made the promise
    // false in only the top few percent of the range, which the sampler
    // reaches rarely enough that a fixed seed missed it every run.
    b.f12 < 100
}

/// Shape 4: `#[non_exhaustive]` on one *variant*, not the enum itself.
pub enum Status {
    Ok,
    #[non_exhaustive]
    Weird {
        code: u32,
    },
}

#[ply::ensures(|result| *result)]
pub fn uses_status(s: Status) -> bool {
    matches!(s, Status::Ok | Status::Weird { .. })
}

/// Shape 5: the type lives in a private module, re-exported through a
/// `pub use` facade -- `Quota` itself is public, but its module
/// (`quota`, declared without `pub` below, in `quota.rs`) is not, so its
/// real path (`quota::Quota`) cannot be named from outside.
mod quota;
pub use quota::Quota;

#[ply::ensures(|result| *result)]
pub fn uses_quota(q: Quota) -> bool {
    q.n == q.n
}

/// An ordinary function sharing this crate's one generated harness with
/// all five shapes above -- FALSE after any call with `n == 0`, so a check
/// that actually ran finds it. If any of the five shapes above broke the
/// shared harness, this promise would report `tool_error`, not `violation`.
#[ply::ensures(|result| *result > 0)]
pub fn normal_fn(n: u32) -> u32 {
    n
}
