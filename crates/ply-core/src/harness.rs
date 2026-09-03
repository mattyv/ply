//! Harness codegen: parses a contracted function's source (the §5.4a subset
//! only enough to render this slice's fixtures -- the full E0501 validator is
//! explicitly out of scope, per the M3 brief) and generates the Kani
//! `proof_for_contract` proof module, including the mandatory unwind
//! emission for `Vec`-typed parameters (§5.4b, measured in
//! docs/m3-slice-findings.md, never left to Kani's default inference).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use indexmap::IndexMap;
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::{Expr, ExprClosure, FnArg, ItemFn, Pat, Type};

/// `ply.yaml`'s `routes:` map, bare type name -> declared function path,
/// exactly [`crate::model::Document::routes`]'s own shape -- passed down
/// from `verify` into every place a struct/enum parameter gets resolved, so
/// a type with no constructor this scan can find on its own still has one
/// more door to try before it is refused (§5.4b's generator hook).
pub type RouteTable = IndexMap<String, String>;

/// The type vocabulary Ply's codegen recognizes. `VecU8` is the only
/// collection shape the *Kani* path (`bounded`) builds (with the mandatory
/// unwind emission, §5.4b) -- `Vec(_)` and `BTreeSet(_)` exist only for the
/// *fuzz* path (M4): proptest can generate any of these without Kani's
/// construction/unwind cost, which is exactly why `BTreeSet` -- one of
/// §5.4b's own measured exclusions -- is fuzz-supported but never
/// bounded-supported (see `is_bounded_supported`/`is_fuzz_supported` below,
/// the routing decision M4's shape-aware defaults depend on). Anything else
/// is `Unsupported` and reported as such (V0505), never silently attempted.
///
/// Deliberately out of scope for M4 (recorded, not silently skipped, per
/// docs/m4-findings.md): struct-typed parameters ("field-by-field" fuzzing).
/// Kani's harness codegen here never supported them either, so adding fuzz
/// support only for structs would create an asymmetry the shape-aware
/// default can't express cleanly; the Kani-excluded acceptance shape uses
/// `BTreeSet` instead (the spec's own alternative: "recursive, or a
/// `BTreeSet`").
#[derive(Clone, PartialEq, Eq)]
pub enum RustType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Bool,
    /// `char` -- §5.4b lists it with the integers as "cheap
    /// unconditionally"; measured 2026-08-25, see docs/post-004-fixes.md.
    Char,
    /// `Option<T>` of a supported type -- §5.4b, same measured tier.
    Option(Box<RustType>),
    /// `Result<T, E>` of supported types -- §5.4b, same measured tier.
    Result(Box<RustType>, Box<RustType>),
    /// `[T; N]` -- §5.4b's **preferred** bounded shape ("generated
    /// harnesses should reach for it first"), cheap with no unwind
    /// annotation because the bound is a compile-time constant. Absent from
    /// the implementation until 2026-08-25, which is why vetting 004's
    /// fragment-first rate-card idiom came back `Unsupported("[u32 ; 4]")`.
    Array(Box<RustType>, u32),
    /// `Vec<u8>` -- the only collection shape the Kani path builds.
    VecU8,
    /// `Vec<T>` for a scalar `T` other than `u8` -- fuzz-only (Kani's
    /// harness codegen here never builds anything but `VecU8`).
    Vec(Box<RustType>),
    /// `BTreeSet<T>` for a scalar `T` -- fuzz-only. §5.4b measured this
    /// shape as intractable for Kani beyond one element; proptest has no
    /// such limit, which is the entire point of the M4 fuzz tier (§1: it
    /// "reaches every signature shape ... §5.4b excludes from `bounded`").
    BTreeSet(Box<RustType>),
    /// `usize`/`isize` -- added 2026-08-27 (measured against the rate-
    /// limiter fixture: `usize` alone was 3 of its 70 public-surface type
    /// uses). Pointer-width, unlike every other integer §5.4b already
    /// supports -- see `scalar_byte_width`'s doc for what that costs.
    Usize,
    Isize,
    /// `std::num::NonZero{U8,U16,U32,U64,Usize,I8,I16,I32,I64,Isize}` --
    /// added 2026-08-27, the rate-limiter fixture's single most common
    /// non-integer type (`NonZeroU32`: 10 of 70 uses measured). Wraps one of
    /// the plain integer variants above -- never itself, and the parser
    /// never constructs one over anything else. The wrapper changes only
    /// *construction*: a `NonZero` is never `kani::any()`'d directly (see
    /// `render_kani_args`) because nothing here trusts Kani's own
    /// `Arbitrary` impl for the type to itself forbid zero -- the task this
    /// shipped under was explicit that this constraint must reach the
    /// solver, not be assumed true by convention. That is also why `NonZero`
    /// is deliberately *not* part of [`RustType::is_leaf`]: nesting it
    /// inside `Option`/`Result`/`[T; N]` would hand construction back to a
    /// generic `kani::any::<T>()` call this module does not control, the
    /// exact risk this type exists to avoid. Only a bare top-level
    /// parameter/return `NonZero` is supported, same as `VecU8`.
    NonZero(Box<RustType>),
    /// `std::time::Duration` -- added 2026-08-27, the rate-limiter
    /// fixture's most common type of all (11 of 70 measured uses on its own
    /// module). A pair of integers (whole seconds, nanoseconds under one
    /// billion) -- never derived as a generic struct, because its fields
    /// are private, so §5.4b's "structs of Ply-derivable Arbitrary (public,
    /// invariant-free fields)" cannot see them at all. Always built through
    /// the public `Duration::new(secs, nanos)` constructor with
    /// `nanos < 1_000_000_000` asserted at construction (see
    /// `render_kani_args`) -- `Duration::new` in fact normalizes a larger
    /// `nanos` by carrying into `secs` rather than panicking, but generating
    /// only pre-normalized values is the honest, auditable choice: a
    /// witness that needed the carry to make sense would be harder to read,
    /// not impossible to build.
    ///
    /// **Measured, not assumed, whether this needs its own bound (task
    /// brief, 2026-08-27):** it does not. `tests/fixtures/durationnonzero`
    /// carries six functions over these new shapes (`Duration`, `NonZeroU32`,
    /// `NonZeroUsize`, `usize`, `isize`), each checked with both `bounded(2)`
    /// and `fuzz(64)` — twelve real engine invocations, no two functions
    /// sharing a cached result. A cold run (`ply.lock` cleared) completed
    /// in 1m26s wall-clock with every one earning a clean `bounded(2)` and
    /// zero diagnostics — about 7s/harness on average, the ordinary
    /// per-invocation cost of a trivial Kani harness of any shape, not a
    /// cost specific to `Duration`. No harness came near the 60s
    /// per-check `--engine-timeout` used for that run. Two independent
    /// `kani::any()` calls plus one `assume`, no loop and so no unwind bound
    /// to surface, unlike `Vec` — the seconds field ranges over the whole
    /// `u64` with no bound at all.
    Duration,
    /// `f32`/`f64` -- added for the sampling/proving split (task,
    /// 2026-08-27). Fuzz-supported, **never** bounded-supported: this is a
    /// deliberate design decision, not a measured Kani exclusion the way
    /// `BTreeSet` is (§5.4b's own list is evidence-backed for the shapes it
    /// names; this one is the split's own point -- "a string is trivial to
    /// sample and genuinely hard to prove ... probably for floats" per the
    /// task brief). Reasoning about floating-point arithmetic exhaustively
    /// (rounding, subnormals, the exact bit-level comparison semantics
    /// CBMC/Kani must model) is real, substantial solver work in a way
    /// sampling never has to pay for -- so rather than spend a session
    /// measuring exactly how bad it is, Ply refuses `bounded`/`proved` on a
    /// float by name (`V0508`) and routes it to `fuzz`/`test` instead,
    /// where it is cheap and honest.
    ///
    /// Deliberately **not** part of [`RustType::is_leaf`] /
    /// `is_composite_constructible`, the same narrowing already applied to
    /// `NonZero`/`Duration`: only a bare top-level parameter or return is
    /// supported, never `Option<f64>`/`Result<f64, _>`/`[f64; N]`/
    /// `Vec<f64>`/`BTreeSet<f64>`. Widening that is possible (the fuzz side
    /// has no technical obstacle to it) but was not attempted here --
    /// nesting bare `is_leaf`-based composability would also have made
    /// `Option<f64>` *bounded*-supported via `is_bounded_supported`'s own
    /// generic composite fallback, which is exactly wrong, and untangling
    /// that cleanly is more surface than one bare shape needs to cover the
    /// task's own required case (a rate limiter's plain `f64` refill
    /// arithmetic).
    ///
    /// **The NaN/infinity decision, made deliberately rather than by
    /// accident (task brief):** Ply's generated float strategy
    /// (`fuzz_gen::strategy_expr`) excludes NaN and both infinities by
    /// default, sampling only ordinary finite floats (ordinary/ normal,
    /// subnormal, zero, either sign). A generated NaN makes almost any
    /// comparison in a postcondition false (`NaN >= x` is false for every
    /// `x`, including `NaN` itself), which would report a broken promise on
    /// an input the real program may never produce -- a false
    /// counterexample, which this project treats as nearly as damaging as a
    /// false pass (both end with the tool switched off, per the task
    /// brief). The choice is not silent: `W0518` (info) names it on every
    /// fuzz/test run over a float-shaped fn, so a user who *does* need NaN/
    /// infinity behaviour checked knows Ply's default run said nothing
    /// about it, rather than discovering that gap by reading source.
    F32,
    F64,
    /// `String` -- added for the sampling/proving split's second headline
    /// case (task, 2026-08-27: "a string is trivial to sample and genuinely
    /// hard to prove"). Fuzz-supported, **never** bounded-supported, by the
    /// same deliberate design decision as `F32`/`F64` -- a `bounded`/`proved`
    /// check on a `String` is refused by name (`V0508`, `bounded_refused_
    /// sample_only_diag` in verify.rs already fires generically off
    /// `is_bounded_supported`/`is_fuzz_supported`, no change needed there).
    /// Kani has no `Arbitrary` for `String` that this codegen builds, and
    /// reasoning about every possible sequence of Unicode scalar values at
    /// once is exactly the kind of open-ended, unbounded-length proof work
    /// this project is not taking on in v1 -- the same "real, substantial
    /// work, not attempted here" reasoning as the float split, never a
    /// measured Kani exclusion the way `BTreeSet` is.
    ///
    /// **Deliberately not part of [`RustType::is_leaf`] /
    /// `is_composite_constructible`**, the same narrowing already applied to
    /// `NonZero`/`Duration`/`F32`/`F64`: only a bare top-level parameter or
    /// return is supported, never `Option<String>`/`Result<String, _>`/
    /// `[String; N]`/`Vec<String>`. Widening that is future work, not
    /// attempted here, for the same reason floats were left un-nested: it
    /// would make e.g. `Option<String>` silently *bounded*-supported via
    /// `is_bounded_supported`'s generic composite fallback, which is exactly
    /// backwards.
    ///
    /// **Also deliberately not folded into `Vec<T>`'s existing scalar
    /// element gate** (`RustType::is_scalar`, unchanged by this task):
    /// `Vec<String>`/`BTreeSet<String>` stay `Unsupported`. The task that
    /// added this type asked for "`Vec<T>` for already-supported `T`", and
    /// the already-existing `Vec`/`BTreeSet` codegen (M4, pre-dating this
    /// task) only ever builds a *scalar* element -- widening element
    /// construction to a second container-shaped type is real, separate
    /// codegen work (a nested nested-strategy, a nested marker encoding)
    /// this pass did not take on. Narrowed, not solved -- see fuzz_gen.rs's
    /// own note.
    ///
    /// **Content and length, made deliberately rather than by accident**
    /// (task brief, mirroring the NaN/infinity precedent): see
    /// `fuzz_gen::strategy_expr`'s own doc comment on its `RustType::String`
    /// arm for the exact choice and why. In short: bounded to at most 32
    /// Unicode characters (never bytes -- multi-byte content is exactly the
    /// point, see below), sampling ordinary printable text -- ASCII
    /// printable characters most of the time, plus real multi-byte Unicode
    /// (accented letters, CJK, symbols) some of the time -- while excluding
    /// ASCII/Latin-1 control characters (`0x00..=0x1F`, `0x7F..=0x9F`) by
    /// default, the same "exclude the class most likely to be a false
    /// alarm" reasoning as the float NaN exclusion: a raw control byte is
    /// the input class real user-facing text is least likely to actually
    /// contain, and is also the value class most likely to trip an
    /// unrelated assumption (a log line, a terminal, a CSV cell) rather
    /// than the function's own logic. Multi-byte Unicode is emphatically
    /// **not** excluded -- unlike a control byte, any valid `String` a real
    /// caller holds can already contain it (Rust's `String` guarantees
    /// valid UTF-8, never "invalid encoding"), and byte-vs-character
    /// confusion (slicing/truncating by byte count instead of by char
    /// count) is precisely the truncation/encoding bug class this type
    /// exists to catch (task brief: "the richest bug territory").
    ///
    /// **A CLI-level disclosure diagnostic analogous to floats' `W0518`
    /// (naming the control-character exclusion the way the run itself
    /// says so) is not wired in this pass**: that disclosure lives in
    /// `crates/ply-cli/src/verify.rs`, which another agent was working in
    /// this same session for an unrelated feature (receiver construction)
    /// and which this task's own scope says not to touch. `ContractFn::
    /// has_string_shape` below is built ready for that wiring (mirroring
    /// `has_float_shape`'s exact shape) -- the choice is fully documented
    /// and pinned by tests at the harness/fuzz_gen level (`strategy_expr`'s
    /// own tests), but a user running `cargo ply verify` does not yet see
    /// an info-level line naming it the way they do for floats. Recorded
    /// here rather than left to be discovered by review.
    String,
    /// This function's own enclosing type, written `Self` in its
    /// signature -- added 2026-08-27 for receiverless associated functions
    /// (constructors), whose return type is almost always `Self` or
    /// `Result<Self, E>`/`Option<Self>`. Deliberately **not** covered by
    /// `is_leaf`/`is_composite_constructible`/`is_bounded_supported`/
    /// `is_fuzz_supported`, which all answer "can Ply *construct* one" --
    /// the question that matters for a parameter. A return value is
    /// produced by the real call, never constructed by Ply, so those
    /// questions do not apply to it; `is_bounded_return_supported`/
    /// `is_fuzz_return_supported` are the ones that do, and both say yes.
    /// If `Self` ever appeared as a *parameter* type instead (rare, but
    /// legal Rust: `fn merge(self, other: Self)`), it must not silently
    /// read as supported there -- constructing an arbitrary receiver-shaped
    /// value is exactly the unsettled work `docs/review-self-construction.md`
    /// describes, not something this variant answers for free.
    SelfType,
    /// `-> ()`, i.e. no declared return type at all -- its own shape rather
    /// than folded into `Unsupported` or treated as absent, so a return-type
    /// gate has one real answer to give for every function, not a special
    /// `None` case its caller has to remember to handle. Return-supported on
    /// both engines, same reasoning as `SelfType`: nothing is ever
    /// constructed from it.
    Unit,
    /// A struct/enum **parameter** Ply builds by calling the type's own
    /// constructor -- rule 1 of `docs/review-self-construction.md`'s
    /// settled order, added for struct/enum parameters (as opposed to a
    /// `&self` receiver, which `ReceiverPlan`/`ContractFn::receiver` already
    /// covered): every value reached this way is one the real program could
    /// build too, through this exact call, so no invariant is assumed.
    /// Literally reuses [`ReceiverPlan`] -- the same constructor-call
    /// mechanism receiver construction built, minus the "bounded sequence of
    /// further operations" half (`operations` is always empty here, and
    /// `max_sequence_len` always 0): a parameter is handed to the real
    /// function once, built, never a `self` further calls run against, so
    /// there is no receiver here to mutate before the call. See the
    /// "Parameter construction" section below for the resolver that
    /// produces this and why reusing `ReceiverPlan` rather than a second
    /// type is the right call.
    UserTypeCtor(Box<ReceiverPlan>),
    /// A struct/enum **parameter** Ply builds directly, field by field
    /// (struct) or variant by variant (enum) -- rule 2: only ever produced
    /// when every field (struct) or every variant's every field (enum) is
    /// already public, so there is no invariant to violate because any
    /// caller could already build any combination. This is §5.4b's own
    /// "structs and enums ... of Ply-derivable Arbitrary (public,
    /// invariant-free fields)", and `docs/review-self-construction.md` is
    /// the reasoning it rests on: "invariant-free" is a **named
    /// assumption** carried on every verdict that rests on it
    /// (`public_fields_assumed_diag` in `verify.rs`), never a proof --
    /// `SweepReport`'s own worked counterexample in that review
    /// (`keys_before - keys_removed == keys_after`, enforced by nothing but
    /// `sweep` itself) is exactly the risk the disclosure names.
    UserTypeFields(Box<UserTypeFieldsPlan>),
    /// `[T]` behind a shared reference (`&[T]`) -- added 2026-09-02
    /// (TODO.md, "make the sampling engine's decision recursive, and add
    /// slices"). Not handled as a shape at all before this: `&[T]` reaches
    /// this parser as `Type::Reference` around `Type::Slice`, and nothing
    /// matched `Type::Slice` on its own, so it fell to the catch-all
    /// `Unsupported`. Built the same way `&Vec<u8>` already is (see
    /// [`RustType::VecU8`]'s own doc): an owned `Vec<T>` is constructed and
    /// lent at the call site as `&name`, relying on `Vec<T>`'s `Deref<Target
    /// = [T]>` to coerce the reference into the `&[T]` the real function
    /// wants -- the exact same trick, no second mechanism. **Fuzz-only**,
    /// like `Vec`/`BTreeSet`: Kani's harness codegen here has never built
    /// anything but `VecU8`, so a slice earns no more than `Vec<T>` already
    /// does on the proof engine -- never bounded-supported, and this is
    /// unchanged from before this task (`&[T]` was `Unsupported`, hence
    /// already not bounded-supported, on both engines).
    Slice(Box<RustType>),
    /// `(A, B, ...)` -- a tuple of supported element types, added for the
    /// same task as [`RustType::Slice`]. Fuzz-only, built element by
    /// element; never bounded-supported (Kani's harness codegen here has
    /// never built a tuple literal either, matching the struct/enum
    /// parameter reasoning `is_bounded_supported`'s own doc already gives).
    Tuple(Vec<RustType>),
    /// `BTreeMap<K, V>` -- the "map" composition shape, added alongside
    /// [`RustType::BTreeSet`] for the same reason: proptest has no trouble
    /// building one of any supported `K`/`V`, and `BTreeMap` (unlike
    /// `HashMap`) needs no extra hasher plumbing and gives deterministic
    /// iteration order, which is what `BTreeSet` was already chosen for.
    /// Fuzz-only, never bounded-supported (Kani's harness codegen here has
    /// never built one, and `BTreeMap::insert` is exactly the same
    /// generic-algorithm shape §5.4b already measured as intractable for
    /// `BTreeSet` past one element).
    BTreeMap(Box<RustType>, Box<RustType>),
    /// `Box<T>` -- an owning wrapper, added for the same task as the three
    /// shapes above. Built by constructing an owned `T` and wrapping it,
    /// `Box::new(value)`. Fuzz-only: `Box` is not in §5.4b's bounded list,
    /// and this task's own standing obligation is that the bounded list
    /// does not change, so it stays unsupported there rather than being
    /// newly added.
    BoxT(Box<RustType>),
    Unsupported(String),
}

/// A struct or enum Ply builds directly because every field/variant-field
/// is public (`RustType::UserTypeFields`'s rule 2) -- the type's bare name
/// (for diagnostics) plus its own shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTypeFieldsPlan {
    pub type_name: String,
    /// `type_name`, qualified from the crate root (`bucket::Quota`, not
    /// bare `Quota`) -- see [`ReceiverPlan::import_path`]'s doc for why
    /// `fuzz_gen::wrap_fn_harness_module` needs this.
    pub import_path: String,
    pub shape: UserTypeShape,
    /// A complete sentence naming a constructor `resolve_user_type` found
    /// for this type but could not use -- `None` when no constructor
    /// candidate existed at all (2026-08-28,
    /// docs/review-structs-enums.md finding 2: "is the disclosure enough?
    /// -- No", because the old wording claimed no constructor existed even
    /// when one did, just not a usable one). `verify`'s
    /// `public_fields_assumed_diag` (W0522) surfaces this alongside the
    /// public-fields assumption it already discloses, rather than staying
    /// silent about a constructor a reader could have written a bug report
    /// about.
    pub skipped_constructor: Option<String>,
}

/// **Deliberately narrow** (2026-08-27, struct/enum parameters): only a
/// named-field struct or a named-field/unit enum variant is recognised here
/// -- a tuple struct or a tuple variant is refused by name (`UserTypeError`)
/// rather than guessed at, the same "narrowed, not solved" discipline
/// `RustType::String`'s own doc already uses for nesting. Every field's own
/// type is resolved the same recursive way a constructor's own argument is
/// (`resolve_user_type`), so a public field that is itself a buildable user
/// type is not refused just because it is not a bare scalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserTypeShape {
    /// A plain struct with named fields, all public.
    Struct(Vec<Param>),
    /// Every variant this enum declares, each with its own name and its own
    /// named fields (empty for a unit variant). **Decided deliberately**
    /// (the task's own question -- "is every variant reachable"): in plain
    /// Rust a variant's fields carry the *enum's* visibility, never their
    /// own, so "all fields public" is really "is every variant's own field
    /// data a shape Ply can build" -- and if even one variant's field is
    /// not, `resolve_user_type` refuses the **whole enum** by name rather
    /// than silently building only the variants it can. A harness that
    /// quietly dropped one variant would under-represent the type without
    /// saying so, which is the "false clean" shape this project refuses on
    /// principle (§1); a named refusal that says which variant and why
    /// costs one sentence and tells the truth. Every variant this rule
    /// *does* admit is one the real program can already construct too
    /// (nothing here invents a field combination no constructor produces --
    /// unlike a struct's own fields, a Rust enum variant's shape is fixed by
    /// its declaration, so there is no "combination" to invent in the first
    /// place), which is what answers the task's other question: generating
    /// any admitted variant is never a false alarm.
    Enum(Vec<(String, Vec<Param>)>),
}

/// Spelled the way the user wrote it, never the way Ply stores it.
///
/// Three diagnostics interpolate a parameter's type with `{:?}` -- the
/// "Ply cannot check this shape" refusals. With the derived `Debug` those
/// read `card_bps: Unsupported("[u32 ; 4]")`, which asks the reader to know
/// what an internal enum variant is before they can find out that their
/// array parameter is the problem. This is the only `Debug` this type ever
/// needed.
impl std::fmt::Debug for RustType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_name())
    }
}

impl RustType {
    /// True for the plain scalar leaf types (never a collection) -- used to
    /// decide whether a `Vec`/`BTreeSet` element type is itself fuzzable.
    pub fn is_scalar(&self) -> bool {
        matches!(
            self,
            RustType::U8
                | RustType::U16
                | RustType::U32
                | RustType::U64
                | RustType::I8
                | RustType::I16
                | RustType::I32
                | RustType::I64
                | RustType::Usize
                | RustType::Isize
                | RustType::Bool
        )
    }

    /// True for exactly the ten integer widths a `NonZero` may wrap
    /// (`u8`..`u64`/`usize`, `i8`..`i64`/`isize` -- never `bool`, and never
    /// another `NonZero`). The parser (`rust_type_from_syn_at`) never
    /// constructs `NonZero` over anything else, but every consumer of
    /// `NonZero` checks this rather than assuming the parser's own
    /// discipline held, so a future change to the parser fails loudly here
    /// instead of producing a `NonZero` codegen cannot build.
    fn is_valid_nonzero_inner(&self) -> bool {
        matches!(
            self,
            RustType::U8
                | RustType::U16
                | RustType::U32
                | RustType::U64
                | RustType::Usize
                | RustType::I8
                | RustType::I16
                | RustType::I32
                | RustType::I64
                | RustType::Isize
        )
    }

    /// The `NonZero{X}` suffix for this integer type (`"U32"`, `"Usize"`,
    /// ...), used to spell `std::num::NonZero{X}` in generated code and
    /// diagnostics. `None` for anything [`is_valid_nonzero_inner`] refuses.
    /// `pub` because codegen outside this module (`contract_rt`,
    /// `engines::kani`, `fuzz_gen`) needs the exact same suffix to render a
    /// `NonZero` witness back out as a literal -- one spelling, not a second
    /// hand-maintained copy of it.
    pub fn nonzero_suffix(&self) -> Option<&'static str> {
        Some(match self {
            RustType::U8 => "U8",
            RustType::U16 => "U16",
            RustType::U32 => "U32",
            RustType::U64 => "U64",
            RustType::Usize => "Usize",
            RustType::I8 => "I8",
            RustType::I16 => "I16",
            RustType::I32 => "I32",
            RustType::I64 => "I64",
            RustType::Isize => "Isize",
            _ => return None,
        })
    }

    /// A type both `kani::any()` and proptest's `any()` build directly with
    /// no construction loop: the scalars plus `char`. Deliberately excludes
    /// `NonZero` and `Duration` even though both are supported top-level
    /// shapes -- see their own doc comments on the enum for why nesting
    /// either inside `Option`/`Result`/`[T; N]` is refused rather than
    /// silently attempted.
    pub fn is_leaf(&self) -> bool {
        self.is_scalar() || matches!(self, RustType::Char)
    }

    /// `Option`/`Result`/`[T; N]` all the way down to leaves. Separated from
    /// `is_leaf` because these carry no unwind cost (an array's length is a
    /// compile-time constant, an `Option` is a two-way branch) while `Vec`
    /// does -- that asymmetry is §5.4b's, measured, not a guess.
    pub fn is_composite_constructible(&self) -> bool {
        match self {
            RustType::Option(inner) | RustType::Array(inner, _) => {
                inner.is_leaf() || inner.is_composite_constructible()
            }
            RustType::Result(ok, err) => {
                (ok.is_leaf() || ok.is_composite_constructible())
                    && (err.is_leaf() || err.is_composite_constructible())
            }
            _ => false,
        }
    }

    /// The narrower gate: can Ply's *Kani* codegen build this type at all?
    /// (Renamed from the M3 slice's `is_supported` now that a second,
    /// broader gate -- `is_fuzz_supported` -- exists; every M3 call site is
    /// updated to this name, behaviour unchanged for every type M3 knew
    /// about.)
    pub fn is_bounded_supported(&self) -> bool {
        match self {
            RustType::VecU8 => true,
            RustType::Vec(_) | RustType::BTreeSet(_) | RustType::Unsupported(_) => false,
            RustType::NonZero(inner) => inner.is_valid_nonzero_inner(),
            RustType::Duration => true,
            // The sampling/proving split's own point (task, 2026-08-27): a
            // float is fuzz-supported (see `is_fuzz_supported` below) but
            // never bounded-supported, by deliberate design decision rather
            // than a measured Kani exclusion -- see the variant's own doc
            // comment. Written as an explicit arm rather than left to the
            // generic fallback below: `is_leaf`/`is_composite_constructible`
            // deliberately never mention `F32`/`F64` either, so the
            // fallback already agrees, but a design decision this load-
            // bearing (it is what makes `V0508` fire at all) is named here
            // rather than left implicit in what two *other* predicates
            // happen not to say.
            RustType::F32 | RustType::F64 => false,
            // Same deliberate-design-decision reasoning as floats, not a
            // measured Kani exclusion: see `RustType::String`'s own doc.
            RustType::String => false,
            // Struct/enum parameters (2026-08-27): never bounded-supported,
            // matching the receiver mechanism they reuse -- Kani's harness
            // codegen has never built a constructor call or a struct/enum
            // literal (`fuzz_gen`'s own module doc, pre-dating this task),
            // and the sequence-of-operations idea a receiver's own `bounded`
            // refusal already cites ("affordable... unmeasured on the
            // exhaustive tier") applies here too. Explicit arm rather than
            // left to the fallback below, matching floats'/`String`'s own
            // reasoning: this is what makes the sample-only refusal fire by
            // name (`bounded_refused_sample_only_diag`) rather than the
            // generic "not attempted" one.
            RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => false,
            other => other.is_leaf() || other.is_composite_constructible(),
        }
    }

    /// Whether a `bounded` proof over this type is a proof over its
    /// *entire* value space -- the domain-coverage condition D5's first
    /// branch (§5.5) needs, found missing by adversarial review 2026-08-25.
    /// A scalar's `kani::any()` ranges over every value the type can hold,
    /// so a callee's `bounded(k)` proof over one already covers whatever a
    /// caller could ever pass; standing on it costs nothing. `VecU8` is the
    /// opposite: its harness only ever builds vectors up to the declared
    /// bound `k`, so the callee's contract is established for lengths
    /// `<= k` and nothing longer -- a caller that passes a longer vector
    /// gets the contract assumed on an input the proof never covered.
    /// Reproduced: a callee honouring its promise only up to length 2, a
    /// caller always passing length 3, composed to a false clean
    /// `bounded(2)` while the real function broke its promise on every
    /// input.
    ///
    /// `[T; N]` is excluded here too, conservatively rather than by
    /// argument: its length is fixed by the type itself, so a caller
    /// cannot pass a differently-sized array at all, which arguably leaves
    /// no containment gap to begin with -- but proving that is exactly the
    /// containment argument this gate exists to avoid needing yet, so it
    /// is narrowed into branch two along with the genuine collection
    /// shapes rather than carved out on unverified reasoning.
    pub fn is_full_domain(&self) -> bool {
        match self {
            RustType::VecU8
            | RustType::Vec(_)
            | RustType::BTreeSet(_)
            | RustType::Array(_, _)
            // Never reached today (a `bounded` proof never runs over a
            // `String` at all -- `is_bounded_supported` is `false`), but
            // correct in its own right for the same reason `Vec` is: Ply's
            // `String` sampling is always length-capped, so standing on a
            // (hypothetical) proof over it would cover only strings up to
            // that cap, never the type's whole value space.
            | RustType::String
            // A struct/enum parameter is built from one constructor call (or
            // one randomly-chosen variant), never every value the type could
            // hold -- the same "sampled, not exhaustive" reasoning as
            // `String`/`Vec` just above, and for the same reason never
            // reached today (`is_bounded_supported` already says `false`).
            | RustType::UserTypeCtor(_)
            | RustType::UserTypeFields(_)
            // The four composition shapes (2026-09-02): never reached
            // today either (none is ever `is_bounded_supported`), and
            // correct for the same "sampled, not exhaustive" reason as
            // `Vec`/`String` above if they ever were -- a slice/list/map is
            // sampled up to a length bound, a tuple/`Box` is exactly as
            // exhaustive (or not) as its own element(s).
            | RustType::Slice(_)
            | RustType::Tuple(_)
            | RustType::BTreeMap(_, _)
            | RustType::BoxT(_)
            | RustType::Unsupported(_) => false,
            RustType::Option(inner) => inner.is_full_domain(),
            RustType::Result(ok, err) => ok.is_full_domain() && err.is_full_domain(),
            _ => true,
        }
    }

    /// The M4 gate: can the *fuzz* (proptest) codegen build this type?
    /// Strictly broader than `is_bounded_supported` -- every Kani-supported
    /// shape is fuzz-supported too.
    ///
    /// **Made a real recursive grammar, 2026-09-02 (TODO.md, "the type wall
    /// has a generic answer").** Before this task, every arm below except
    /// `Vec`/`BTreeSet` answered a *bare* type (a `String`, a float, a user
    /// struct), and composition was refused one constructor at a time:
    /// `Option`/`Result`/`[T; N]` fell through to `is_bounded_supported`'s
    /// own fallback, which asks `is_composite_constructible` -- a question
    /// that only ever recurses into `is_leaf`/itself, never into any of the
    /// arms *this* function adds, so `Option<String>` inherited `false` from
    /// the *proof* engine's own narrower answer even though a `String` is
    /// fuzz-supported alone. That is the defect this task measures: every
    /// shape below is buildable by itself, and none of them could nest
    /// inside another. Composition closes that: `Option`, `Result`, a fixed
    /// array, a list, a set, a map, a slice, a tuple, and an owning wrapper
    /// (`Box`) are now each answered by recursing into this same function
    /// for their own element(s), so *any* fuzz-supported element (a plain
    /// scalar, a `String`, a float, a `NonZero`/`Duration`, a user struct,
    /// or another composed shape) may nest inside any of them, to any
    /// depth. Only the leaf answers themselves are unchanged.
    pub fn is_fuzz_supported(&self) -> bool {
        match self {
            RustType::Unsupported(_) => false,
            RustType::Option(inner) | RustType::Array(inner, _) => {
                inner.is_fuzz_supported() && inner.is_fuzz_nestable()
            }
            RustType::Result(ok, err) => {
                ok.is_fuzz_supported()
                    && ok.is_fuzz_nestable()
                    && err.is_fuzz_supported()
                    && err.is_fuzz_nestable()
            }
            // `Vec`/`BTreeSet` no longer stop at a scalar element -- any
            // fuzz-supported inner composes, the parser's own widened gate
            // (`is_composable`) already lets any such inner reach here.
            RustType::Vec(inner) | RustType::BTreeSet(inner) => {
                inner.is_fuzz_supported() && inner.is_fuzz_nestable()
            }
            // The four brand-new composition shapes (TODO.md, "add
            // slices" and "make the sampling engine's decision recursive"):
            // each answered the same recursive way as `Vec`/`Option` above.
            RustType::Slice(inner) | RustType::BoxT(inner) => {
                inner.is_fuzz_supported() && inner.is_fuzz_nestable()
            }
            RustType::Tuple(items) => items
                .iter()
                .all(|t| t.is_fuzz_supported() && t.is_fuzz_nestable()),
            RustType::BTreeMap(key, value) => {
                key.is_fuzz_supported()
                    && key.is_fuzz_nestable()
                    && value.is_fuzz_supported()
                    && value.is_fuzz_nestable()
            }
            // The one type this gate says `true` for where
            // `is_bounded_supported` says `false` for a reason other than a
            // measured Kani exclusion (`BTreeSet`/`Vec` fall out of the
            // `other => other.is_bounded_supported()` arm below just fine,
            // since *they* are never fuzz-*and*-bounded-disagreeing at the
            // bare-scalar level) -- floats need their own arm precisely
            // because the fallback would otherwise inherit `false` from
            // `is_bounded_supported`, which is backwards: proptest builds an
            // arbitrary `f32`/`f64` as cheaply as any other scalar.
            RustType::F32 | RustType::F64 => true,
            // The same reason floats need their own arm here: the fallback
            // would otherwise inherit `false` from `is_bounded_supported`,
            // which is backwards -- proptest builds a bounded-length,
            // curated-content `String` as cheaply as any other sampled
            // shape (see `fuzz_gen::strategy_expr`'s own doc for what it
            // actually builds).
            RustType::String => true,
            // Struct/enum parameters (2026-08-27): the whole point of this
            // shape -- built via a constructor call or a direct field/
            // variant literal, both ordinary imperative codegen the fuzz
            // harness crate can compile just fine (it is a real crate
            // depending on the target one, so it can call the target's own
            // public constructor or name its own public fields, exactly as
            // any other downstream crate could). Same reason floats/
            // `String` need their own arm: the fallback would otherwise
            // inherit `false` from `is_bounded_supported`, which is
            // backwards here too.
            RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => true,
            other => other.is_bounded_supported(),
        }
    }

    /// Whether `self` is safe to *nest* inside another shape's own sampled
    /// value -- narrower than [`is_fuzz_supported`](Self::is_fuzz_supported)
    /// for a struct/enum specifically, where that method always says `true`
    /// (correct for a *top-level* parameter, which has the full
    /// `fuzz_gen::plan_for_param`/`__ply_rejected` rejection mechanism
    /// available). Nested composition builds a struct/enum from a raw leaf
    /// tuple with ordinary Rust code (`fuzz_gen::construct_from_raw_expr`),
    /// which has no access to proptest's own case-rejection partway through
    /// a container -- so a constructor carrying its own `#[ply::requires]`
    /// filter, or one with a fallible (`Result<Self, E>`) return, is refused
    /// **only when reached this way**, even though the exact same type is
    /// fine as a bare top-level parameter. Every other shape recurses
    /// through its own element(s), since a `requires`-gated constructor
    /// buried two containers deep is exactly as unrenderable as one nested
    /// directly.
    pub fn is_fuzz_nestable(&self) -> bool {
        match self {
            RustType::UserTypeCtor(plan) => {
                plan.ctor_requires.is_none()
                    && matches!(plan.ctor_return, CtorReturn::Bare)
                    && plan.ctor_params.iter().all(|p| p.ty.is_fuzz_nestable())
            }
            RustType::UserTypeFields(plan) => match &plan.shape {
                UserTypeShape::Struct(fields) => fields.iter().all(|f| f.ty.is_fuzz_nestable()),
                UserTypeShape::Enum(variants) => variants
                    .iter()
                    .all(|(_, fields)| fields.iter().all(|f| f.ty.is_fuzz_nestable())),
            },
            RustType::Option(inner)
            | RustType::Array(inner, _)
            | RustType::Vec(inner)
            | RustType::BTreeSet(inner)
            | RustType::Slice(inner)
            | RustType::BoxT(inner) => inner.is_fuzz_nestable(),
            RustType::Result(ok, err) => ok.is_fuzz_nestable() && err.is_fuzz_nestable(),
            RustType::Tuple(items) => items.iter().all(RustType::is_fuzz_nestable),
            RustType::BTreeMap(key, value) => key.is_fuzz_nestable() && value.is_fuzz_nestable(),
            _ => true,
        }
    }

    /// Whether this type -- at any depth, top-level or nested inside a
    /// composed shape -- includes a value built through §5.4b's declared
    /// route. `verify` uses this (rather than only the runtime distinct-
    /// value marker, which the degenerate-route guard deliberately narrows
    /// to a *top-level* parameter) to decide whether a run's evidence earns
    /// the `route-built` mark: the mark is a plain fact about where the
    /// values came from, and it is true regardless of how deep the
    /// route-built value sits.
    pub fn uses_any_route(&self) -> bool {
        match self {
            RustType::UserTypeCtor(plan) => {
                plan.route.is_some() || plan.ctor_params.iter().any(|p| p.ty.uses_any_route())
            }
            RustType::UserTypeFields(plan) => match &plan.shape {
                UserTypeShape::Struct(fields) => fields.iter().any(|f| f.ty.uses_any_route()),
                UserTypeShape::Enum(variants) => variants
                    .iter()
                    .any(|(_, fields)| fields.iter().any(|f| f.ty.uses_any_route())),
            },
            RustType::Option(inner)
            | RustType::Array(inner, _)
            | RustType::Vec(inner)
            | RustType::BTreeSet(inner)
            | RustType::Slice(inner)
            | RustType::BoxT(inner) => inner.uses_any_route(),
            RustType::Result(ok, err) => ok.uses_any_route() || err.uses_any_route(),
            RustType::Tuple(items) => items.iter().any(RustType::uses_any_route),
            RustType::BTreeMap(key, value) => key.uses_any_route() || value.uses_any_route(),
            _ => false,
        }
    }

    /// The *return*-position gate (added 2026-08-27, for a receiverless
    /// associated function's return type -- adversarial review of the
    /// method-resolution task; **removed 2026-09-01**, both engines,
    /// measured -- see the retraction below the history). Unlike a
    /// parameter, a return value is never *constructed* by Ply, so
    /// `SelfType`/`Unit` were always fine here -- `is_bounded_supported`/
    /// `is_fuzz_supported` correctly say `false` for them (a *parameter* of
    /// type `Self` really would need construction Ply cannot do), which was
    /// the wrong answer for a return, so this was a genuinely different
    /// question, never an alias.
    ///
    /// **What this gate used to do, kept as history rather than deleted:**
    /// reject an ordinary struct/enum return type Ply's parser does not
    /// model (`Bucket`, returned from outside its own `impl` block) with
    /// `unsupported`, as the review that requested this gate asked -- but
    /// its own doc already conceded, empirically, that once the *actual*
    /// broken-harness cause was fixed (a zero-parameter fn's fuzz strategy
    /// was a bare `()`, not a `Strategy` -- `combined_strategy_expr`'s own
    /// doc), such a return type did not itself break anything on either
    /// engine: nothing in this codegen ever names or constructs a return
    /// type. It stood anyway as a deliberate, requested narrowing --
    /// refusing a shape Ply cannot yet reason about by name, on principle --
    /// matching §5.4b's then-stated rule that a supported signature covers
    /// "parameters *and* return type".
    ///
    /// **Retracted 2026-09-01, measured rather than argued
    /// (docs/reach-measurement-2.md, The-Ply-Spec.md §5.4b).** Run against
    /// a real library (`semver`), this gate alone blocked 10 of 16
    /// independently-written properties -- the single largest blocker after
    /// `&str` parameters. Measured directly before removing it, on both
    /// engines, with a function returning `std::cmp::Ordering` (a type Ply
    /// models nowhere): the fuzz engine earned `fuzzed(64)` on a correct
    /// body and a real `violation` with a shrunk failing input on a broken
    /// one; the bounded (Kani) engine earned a genuine `bounded(2)` proof
    /// on the correct body (not a timeout mislabeled -- it completed in
    /// seconds) and a real `violation` with a concrete counterexample on
    /// the same broken one. Both engines pay this gate's cost for nothing
    /// in return, so it is gone on both, not narrowed to one -- unlike the
    /// parameter side of §5.4b, where the two engines' supported-type lists
    /// genuinely do differ (`bounded`/`fuzz` are no longer the same
    /// question once a value must be *constructed*, which is exactly what
    /// a return value never is).
    pub fn is_bounded_return_supported(&self) -> bool {
        true
    }

    /// The fuzz engine's counterpart to `is_bounded_return_supported` --
    /// same retraction, same measurement, same answer.
    pub fn is_fuzz_return_supported(&self) -> bool {
        true
    }

    /// The exact source text used both to declare `let name: <ty> = ...`
    /// and to decode a scalar witness's byte width.
    pub fn scalar_rust_name(&self) -> Option<&'static str> {
        Some(match self {
            RustType::U8 => "u8",
            RustType::U16 => "u16",
            RustType::U32 => "u32",
            RustType::U64 => "u64",
            RustType::I8 => "i8",
            RustType::I16 => "i16",
            RustType::I32 => "i32",
            RustType::I64 => "i64",
            RustType::Usize => "usize",
            RustType::Isize => "isize",
            RustType::Bool => "bool",
            RustType::F32 => "f32",
            RustType::F64 => "f64",
            _ => return None,
        })
    }

    /// The full type source text, for `let x: <ty> = kani::any();` and for
    /// proptest's `any::<<ty>>()`. `None` for the shapes built by a
    /// dedicated codegen path instead (`Vec`, `BTreeSet`) and for
    /// `Unsupported`.
    pub fn rust_name(&self) -> Option<String> {
        Some(match self {
            RustType::Char => "char".to_string(),
            RustType::Option(inner) => format!("Option<{}>", inner.rust_name()?),
            RustType::Result(ok, err) => {
                format!("Result<{}, {}>", ok.rust_name()?, err.rust_name()?)
            }
            RustType::Array(inner, n) => format!("[{}; {}]", inner.rust_name()?, n),
            // Fully qualified so generated code never depends on a `use`
            // being in scope at the point it is spliced in.
            RustType::Duration => "std::time::Duration".to_string(),
            RustType::NonZero(inner) => {
                format!("std::num::NonZero{}", inner.nonzero_suffix()?)
            }
            RustType::String => "String".to_string(),
            other => other.scalar_rust_name()?.to_string(),
        })
    }

    /// A human-facing spelling of this type, for diagnostics. Unlike
    /// [`RustType::rust_name`] this is total: every shape gets one, because
    /// a diagnostic that names a parameter and then omits its type ("Ply
    /// cannot spell `xs: `") is worse than one that never named it. Kept
    /// separate from `rust_name` on purpose -- that one answers "can codegen
    /// write this type into generated source", which is a different
    /// question with a legitimate `None`.
    pub fn display_name(&self) -> String {
        match self {
            RustType::Char => "char".to_string(),
            RustType::Option(inner) => format!("Option<{}>", inner.display_name()),
            RustType::Result(ok, err) => {
                format!("Result<{}, {}>", ok.display_name(), err.display_name())
            }
            RustType::Array(inner, n) => format!("[{}; {}]", inner.display_name(), n),
            RustType::VecU8 => "Vec<u8>".to_string(),
            RustType::Vec(inner) => format!("Vec<{}>", inner.display_name()),
            RustType::BTreeSet(inner) => format!("BTreeSet<{}>", inner.display_name()),
            RustType::Duration => "Duration".to_string(),
            RustType::NonZero(inner) => {
                format!("NonZero{}", inner.nonzero_suffix().unwrap_or("?"))
            }
            RustType::String => "String".to_string(),
            // The source text as the user wrote it: for a shape Ply does not
            // model, the words they typed are the only spelling that helps.
            RustType::Unsupported(src) => src.clone(),
            RustType::SelfType => "Self".to_string(),
            RustType::Unit => "()".to_string(),
            RustType::UserTypeCtor(plan) => plan.type_name.clone(),
            RustType::UserTypeFields(plan) => plan.type_name.clone(),
            RustType::Slice(inner) => format!("[{}]", inner.display_name()),
            RustType::Tuple(items) => format!(
                "({})",
                items
                    .iter()
                    .map(RustType::display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            RustType::BTreeMap(key, value) => {
                format!("BTreeMap<{}, {}>", key.display_name(), value.display_name())
            }
            RustType::BoxT(inner) => format!("Box<{}>", inner.display_name()),
            other => other.scalar_rust_name().unwrap_or("?").to_string(),
        }
    }

    /// Can a failing input of this type be written back out as a Rust
    /// literal? That is what turns a witness into the runnable `#[test]`
    /// D7 calls the repair target; when it cannot be, `W0541` says so and
    /// reports the engine's own rendering instead of inventing one.
    pub fn is_witness_renderable(&self) -> bool {
        match self {
            RustType::VecU8 => true,
            RustType::Vec(inner) => inner.as_ref() == &RustType::U8,
            RustType::Duration => true,
            RustType::NonZero(inner) => inner.scalar_byte_width().is_some(),
            // Not yet (2026-08-27): a shrunk struct/enum witness is reported
            // witness-only (`W0541`), same as `String`/a float -- Ply has no
            // literal-rendering path for "call this constructor with these
            // values" or "build this variant" yet, only a *description* of
            // it (`marker_display_expr`'s own new arm, used for the
            // human-readable failing-case line, which is a different,
            // already-solved question from "can this become a runnable
            // `#[test]`").
            RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => false,
            _ => self.scalar_byte_width().is_some(),
        }
    }

    /// Byte width Kani's concrete-playback encodes this scalar as
    /// (little-endian on the pinned toolchain's target -- measured, see
    /// docs/m3-slice-findings.md).
    ///
    /// `usize`/`isize` are **pointer-width**, unlike every other integer
    /// here: `Some(8)` is only correct for a 64-bit target. The pinned
    /// toolchain's own default target (and every target this workspace
    /// builds or tests on) is `x86_64`, and §5.2a already records the build
    /// target as part of what a stored result stood on -- a `usize` proof
    /// earned on this target is not evidence for a 32-bit one, exactly the
    /// same honesty condition `unstable_flags` already carries for the
    /// engine's own version. Widening this to read the real target's
    /// pointer width (rather than assuming 8) is future work if Ply ever
    /// runs its engines against a 32-bit target; nothing here claims that
    /// case is covered.
    pub fn scalar_byte_width(&self) -> Option<usize> {
        match self {
            RustType::U8 | RustType::I8 | RustType::Bool => Some(1),
            RustType::U16 | RustType::I16 => Some(2),
            RustType::U32 | RustType::I32 => Some(4),
            RustType::U64 | RustType::I64 => Some(8),
            RustType::Usize | RustType::Isize => Some(8),
            // No witness decoder yet for these -- a violation on one is
            // reported honestly as a tool error rather than with an
            // invented input (see `verify::run_bounded_check`).
            RustType::Char
            | RustType::Option(_)
            | RustType::Result(..)
            | RustType::Array(..)
            | RustType::VecU8
            | RustType::Vec(_)
            | RustType::BTreeSet(_)
            | RustType::NonZero(_)
            | RustType::Duration
            // No byte-width witness decoder for floats either -- a fuzz-
            // found float violation is reported witness-only (`W0541`),
            // never with an invented input, same as `Option`/`Result`/etc.
            // above and for the same reason: nothing here has a Kani
            // witness decoder to reuse, since a float never reaches the
            // bounded/Kani path at all (`is_bounded_supported` is `false`).
            | RustType::F32
            | RustType::F64
            // Same reason as the float arms just above: `String` never
            // reaches the bounded/Kani path at all (`is_bounded_supported`
            // is `false`), so a fuzz-found violation on one is reported
            // witness-only (`W0541`), never with an invented input.
            | RustType::String
            | RustType::SelfType
            | RustType::Unit
            // Same reasoning as `String`/floats just above: never reaches
            // the bounded/Kani path (`is_bounded_supported` is `false`), so
            // no byte-width witness decoder is needed.
            | RustType::UserTypeCtor(_)
            | RustType::UserTypeFields(_)
            // Same reasoning as the struct/enum arms just above -- none of
            // these four composition shapes (added 2026-09-02) ever reaches
            // the bounded/Kani path (`is_bounded_supported` is `false` for
            // all of them), so there is no byte-width witness decoder to
            // give one.
            | RustType::Slice(_)
            | RustType::Tuple(_)
            | RustType::BTreeMap(_, _)
            | RustType::BoxT(_)
            | RustType::Unsupported(_) => None,
        }
    }
}

/// Type aliases declared at the top level of the file being read
/// (`type AccountId = u64;`). §5.4b says nothing about aliases because they
/// are transparent in Rust -- but the extractor matched on the *written*
/// name, so `account: ledger::AccountId` came back
/// `Unsupported("ledger :: AccountId")` and one line of ordinary Rust moved
/// a function out of the checkable set (vetting 004 finding 5).
pub type AliasMap = std::collections::BTreeMap<String, Type>;

/// Depth cap for alias chasing: a cyclic `type A = B; type B = A;` does not
/// compile, but this reader is not a compiler and must not hang on one.
const MAX_ALIAS_DEPTH: usize = 8;

/// Reads one rendered type source (`u8`, `& Vec < u8 >`) back into a
/// [`RustType`]. References are looked through: what matters for building an
/// arbitrary value is the type behind the `&`. Returns `None` when the text
/// is not a Rust type at all.
pub fn rust_type_from_source(src: &str) -> Option<RustType> {
    let ty: Type = syn::parse_str(src).ok()?;
    Some(rust_type_from_syn(&ty, &AliasMap::new()))
}

fn rust_type_from_syn(ty: &Type, aliases: &AliasMap) -> RustType {
    rust_type_from_syn_at(ty, aliases, 0)
}

/// Bare `Self`, with no generic arguments and no `<T as Trait>::` qualifier
/// -- the only spelling `Self` can legally take as a whole type (never
/// `Self<T>`, never part of a longer path).
fn is_bare_self_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp)
        if tp.qself.is_none()
            && tp.path.segments.len() == 1
            && tp.path.segments[0].ident == "Self"
            && tp.path.segments[0].arguments.is_empty())
}

/// Whether a receiverless associated fn's return type is a shape
/// [`scan_ctor_candidates`] (the parameter path) and [`scan_file_for_receiver`]
/// (the receiver path) both accept as a usable constructor -- one shared
/// answer, computed here once, rather than each scan asking its own
/// narrower question: bare `Self`, or -- widened 2026-08-28,
/// docs/review-structs-enums.md finding 2, "a violation reported on correct
/// code" -- `Result<Self, E>`, the ordinary fallible-constructor shape
/// (`Range::new(lo, hi) -> Result<Self, String>`, rejecting `lo > hi`).
/// Before that widening, a `Result`-returning constructor was invisible to
/// every constructor scan, so a type with one and nothing else fell
/// straight through to rule 2 (direct field construction) and Ply built
/// exactly the state the constructor exists to forbid, then reported the
/// function that reads it as violating its own promise.
///
/// The widening reached the parameter path's own scan in that fix but not
/// the receiver path's, which kept its own separate, narrower check until
/// 2026-08-31 (defect 1, docs/reach-measurement-2.md): the exact same
/// constructor Ply had just used to build a *parameter* was reported as not
/// existing when the receiver scan asked. The receiver path now calls this
/// same function rather than carrying a second answer that could disagree
/// with it.
///
/// Deliberately narrow, matching `return_rust_type_from_syn`'s own doc:
/// `Result<Self, E>` behind a type alias, or nested inside another
/// `Option`/`Result`, is not recognised -- only the bare, written shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtorReturn {
    /// `fn new(..) -> Self`.
    Bare,
    /// `fn new(..) -> Result<Self, E>` -- calling it can fail, and a case
    /// where it does is not a usable value, so codegen must reject that
    /// draw rather than treat the `Err` as if it were one.
    ResultSelf,
}

/// Whether a constructor's return type names the type being constructed:
/// `Self`, or the type's own name written out (`-> Quota`, `-> super::Quota`,
/// `-> crate::bucket::Quota`).
///
/// Spelling the type instead of `Self` is ordinary Rust and common in
/// submodule-split crates, but only `Self` was recognised, so such a
/// constructor was invisible and Ply reported "it has no constructor Ply
/// can call" about a type with a public `new` (2026-08-28, found probing
/// the same family as the qualified-`impl` fix).
///
/// A written-out name is accepted only when it resolves to the type's own
/// canonical declaration, the same `Confirmed`-only rule the `impl` block
/// itself is held to: another module's same-named type is a different type,
/// and constructing the wrong one is worse than refusing.
fn return_names_this_type(
    ty: &Type,
    type_name: &str,
    file_mod: &[String],
    use_aliases: &std::collections::BTreeMap<String, Vec<String>>,
    target_absolute: Option<&[String]>,
) -> bool {
    if is_bare_self_type(ty) {
        return true;
    }
    classify_impl_self_ty(ty, type_name, file_mod, use_aliases, target_absolute)
        == ImplMatch::Confirmed
}

fn ctor_return_kind(
    output: &syn::ReturnType,
    _aliases: &AliasMap,
    type_name: &str,
    file_mod: &[String],
    use_aliases: &std::collections::BTreeMap<String, Vec<String>>,
    target_absolute: Option<&[String]>,
) -> Option<CtorReturn> {
    let syn::ReturnType::Type(_, ty) = output else {
        return None;
    };
    let names_it =
        |t: &Type| return_names_this_type(t, type_name, file_mod, use_aliases, target_absolute);
    if names_it(ty) {
        return Some(CtorReturn::Bare);
    }
    if let Type::Path(tp) = ty.as_ref()
        && tp.qself.is_none()
        && let Some(seg) = tp.path.segments.last()
        && seg.ident == "Result"
        && let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
    {
        let args: Vec<&Type> = ab
            .args
            .iter()
            .filter_map(|a| match a {
                syn::GenericArgument::Type(t) => Some(t),
                _ => None,
            })
            .collect();
        if args.len() == 2 && names_it(args[0]) {
            return Some(CtorReturn::ResultSelf);
        }
    }
    None
}

/// Classifies a function's *return* type -- never a parameter's, and a
/// genuinely different question from `rust_type_from_syn` even though it
/// reuses that parser for every shape but one. A parameter must be
/// something Ply can *construct*; a return value never is (the real call
/// produces it), so the one shape that needs its own answer here is `Self`:
/// `RustType::SelfType`, which every `is_bounded_return_supported`/
/// `is_fuzz_return_supported` check treats as fine regardless of the
/// enclosing type's own shape (added 2026-08-27, for the receiverless
/// associated functions -- constructors -- this task's own review flagged
/// as the measurable win; every one of them returns bare `Self`).
///
/// Everything else -- including an *ordinary* struct or enum name that is
/// not `Self` (`Bucket`, returned from outside its own `impl` block) --
/// is classified exactly as a parameter's type would be, so it reports
/// `Unsupported` here too, refused by name rather than silently attempted.
/// **Deliberately narrow**: `Self` nested inside `Option`/`Result`
/// (`Result<Self, ConfigError>`, a real shape in the rate-limiter fixture)
/// is not recognised -- it falls through to the ordinary parser, which has
/// no `Self` case at all, and reports `Unsupported`. Widening this is
/// future work; nothing in this task's own required tests needs it, and
/// overclaiming it without a fixture pinning it is exactly the kind of gap
/// this project's own testing rule (§9) exists to catch before a reviewer
/// does.
pub fn return_rust_type_from_syn(output: &syn::ReturnType, aliases: &AliasMap) -> RustType {
    match output {
        syn::ReturnType::Default => RustType::Unit,
        syn::ReturnType::Type(_, ty) if is_bare_self_type(ty) => RustType::SelfType,
        syn::ReturnType::Type(_, ty) => rust_type_from_syn(ty, aliases),
    }
}

/// A type rendered back to something a person wrote. `to_token_stream()`
/// separates every token, so `impl Into<u32>` comes back as
/// `impl Into < u32 >` and `Vec<u8>` as `Vec < u8 >` -- accurate, and not a
/// spelling any reader recognises. Diagnostics quote these strings
/// directly, and the newbie bar applies to every one of them.
fn normalise_type_source(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c != ' ' {
            out.push(c);
            continue;
        }
        // Drop a space that sits next to punctuation on either side.
        let before = out.chars().last();
        let after = chars.peek().copied();
        let tight = |ch: Option<char>| {
            matches!(
                ch,
                Some('<' | '>' | ',' | '(' | ')' | '[' | ']' | ':' | '&')
            )
        };
        if tight(before) || tight(after) {
            continue;
        }
        out.push(c);
    }
    out
}

fn rust_type_from_syn_at(ty: &Type, aliases: &AliasMap, depth: usize) -> RustType {
    match ty {
        Type::Array(arr) => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(n),
                ..
            }) = &arr.len
            else {
                return RustType::Unsupported(normalise_type_source(
                    &ty.to_token_stream().to_string(),
                ));
            };
            let Ok(n) = n.base10_parse::<u32>() else {
                return RustType::Unsupported(normalise_type_source(
                    &ty.to_token_stream().to_string(),
                ));
            };
            let elem = rust_type_from_syn_at(&arr.elem, aliases, depth);
            if is_composable(&elem) {
                RustType::Array(Box::new(elem), n)
            } else {
                RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
            }
        }
        Type::Path(tp) => {
            let Some(seg) = tp.path.segments.last() else {
                return RustType::Unsupported(normalise_type_source(
                    &ty.to_token_stream().to_string(),
                ));
            };
            // An alias resolves to whatever it names, by its last segment
            // (`ledger::AccountId` and `AccountId` are the same alias).
            if depth < MAX_ALIAS_DEPTH
                && seg.arguments.is_empty()
                && let Some(aliased) = aliases.get(&seg.ident.to_string())
            {
                return rust_type_from_syn_at(aliased, aliases, depth + 1);
            }
            match seg.ident.to_string().as_str() {
                "char" => RustType::Char,
                "Option" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
                        && let Some(syn::GenericArgument::Type(inner_ty)) = ab.args.first()
                    {
                        let inner = rust_type_from_syn_at(inner_ty, aliases, depth);
                        if is_composable(&inner) {
                            return RustType::Option(Box::new(inner));
                        }
                    }
                    RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
                }
                "Result" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        let args: Vec<&Type> = ab
                            .args
                            .iter()
                            .filter_map(|a| match a {
                                syn::GenericArgument::Type(t) => Some(t),
                                _ => None,
                            })
                            .collect();
                        if args.len() == 2 {
                            let ok = rust_type_from_syn_at(args[0], aliases, depth);
                            let err = rust_type_from_syn_at(args[1], aliases, depth);
                            if is_composable(&ok) && is_composable(&err) {
                                return RustType::Result(Box::new(ok), Box::new(err));
                            }
                        }
                    }
                    RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
                }
                "u8" => RustType::U8,
                "u16" => RustType::U16,
                "u32" => RustType::U32,
                "u64" => RustType::U64,
                "i8" => RustType::I8,
                "i16" => RustType::I16,
                "i32" => RustType::I32,
                "i64" => RustType::I64,
                // Pointer-width -- see `scalar_byte_width`'s doc for the
                // honesty condition this carries (a proof built on this
                // target's byte width is not evidence for a different one).
                "usize" => RustType::Usize,
                "isize" => RustType::Isize,
                "bool" => RustType::Bool,
                // Fuzz-supported, never bounded-supported -- see
                // `RustType::F32`/`F64`'s own doc comment for why this is a
                // deliberate design decision, not a measured Kani exclusion.
                "f32" => RustType::F32,
                "f64" => RustType::F64,
                // `std::num::NonZero{X}` -- matched on the bare last
                // segment, so both `std::num::NonZeroU32` (never `use`d)
                // and a bare `NonZeroU32` (after a `use` -- what every
                // fixture and the rate-limiter measurement actually write)
                // resolve to the same shape, the same way `"u8"` above
                // matches regardless of whether it came through a `use` or
                // a fully-qualified path (there is no fully-qualified `u8`,
                // but `Duration` below faces the identical question).
                // Deliberately never generic (`NonZero<u32>`) -- that is
                // not the type's own surface syntax; only the ten
                // `NonZero{X}` names below exist in `std`.
                "NonZeroU8" => RustType::NonZero(Box::new(RustType::U8)),
                "NonZeroU16" => RustType::NonZero(Box::new(RustType::U16)),
                "NonZeroU32" => RustType::NonZero(Box::new(RustType::U32)),
                "NonZeroU64" => RustType::NonZero(Box::new(RustType::U64)),
                "NonZeroUsize" => RustType::NonZero(Box::new(RustType::Usize)),
                "NonZeroI8" => RustType::NonZero(Box::new(RustType::I8)),
                "NonZeroI16" => RustType::NonZero(Box::new(RustType::I16)),
                "NonZeroI32" => RustType::NonZero(Box::new(RustType::I32)),
                "NonZeroI64" => RustType::NonZero(Box::new(RustType::I64)),
                "NonZeroIsize" => RustType::NonZero(Box::new(RustType::Isize)),
                // `std::time::Duration`, matched the same way -- named by
                // its bare last segment, so this is deliberately as
                // vulnerable to a same-named unrelated type as `"u8"` above
                // already is; §5.4b's whole fragment reads source text, not
                // resolved item paths, and widening that is out of scope
                // for this task.
                "Duration" => RustType::Duration,
                // `std::string::String`, matched the same bare-last-segment
                // way as `Duration`/`NonZero{X}` above -- both a bare
                // `String` (after `use`, or the prelude, which always
                // brings it into scope) and a fully-qualified
                // `std::string::String` resolve to the same shape.
                // Deliberately never nested (`Option<String>` etc. fall
                // through to `Unsupported` below, same narrowing as
                // `NonZero`/`Duration`/`F32`/`F64` -- see `RustType::
                // String`'s own doc for why).
                "String" => RustType::String,
                // `&str` reaches here as bare `str`, because references are
                // looked through above. Sampling one is the same job as
                // sampling a `String` -- proptest builds the `String` and the
                // call site passes a reference to it -- so it maps to the
                // same type rather than getting a second one that would then
                // need its own arm in every match over `RustType`.
                //
                // Measured 2026-08-30 as the single largest reason Ply could
                // not check a real library: text parameters blocked 11 of the
                // 16 properties `semver`'s author wrote down
                // (`docs/reach-measurement-2.md`). `String` was already
                // sampled; only the borrowed spelling was refused, which is
                // the spelling almost every real signature uses.
                //
                // Bare `str` is not a type a parameter can have in Rust (it
                // is unsized), so mapping it here can only ever be reached
                // through a reference.
                "str" => RustType::String,
                // Composition (2026-09-02, TODO.md) widened `Vec`'s own
                // element gate past a plain scalar: any element Ply can
                // already build alone (a `String`, a user struct, another
                // container) now composes too, for the *sampling* engine --
                // `is_scalar` still decides the *bounded* (Kani) engine's
                // own, narrower `is_bounded_supported`/`is_full_domain`
                // answer for a `Vec`, unaffected by this parser change
                // (Kani's harness codegen here has never built anything but
                // `VecU8`, so a wider element was never going to change what
                // it builds).
                "Vec" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
                        && let Some(syn::GenericArgument::Type(inner_ty)) = ab.args.first()
                    {
                        if let Type::Path(inner) = inner_ty
                            && inner.path.is_ident("u8")
                        {
                            return RustType::VecU8;
                        }
                        let inner = rust_type_from_syn_at(inner_ty, aliases, depth);
                        if is_composable(&inner) {
                            return RustType::Vec(Box::new(inner));
                        }
                    }
                    RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
                }
                // Fuzz-only (§5.4b measured exclusion): proptest has no
                // trouble generating a BTreeSet of scalars; Kani does, past
                // one element, at any bound. Widened past a scalar element
                // the same way `Vec` just above is -- sampling only.
                "BTreeSet" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
                        && let Some(syn::GenericArgument::Type(inner_ty)) = ab.args.first()
                    {
                        let inner = rust_type_from_syn_at(inner_ty, aliases, depth);
                        if is_composable(&inner) {
                            return RustType::BTreeSet(Box::new(inner));
                        }
                    }
                    RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
                }
                // The "map" composition shape (2026-09-02, TODO.md) --
                // added alongside `BTreeSet` above, for the same reason:
                // deterministic ordering, no extra hasher plumbing. Sampling
                // only, same as `BTreeSet`.
                "BTreeMap" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        let args: Vec<&Type> = ab
                            .args
                            .iter()
                            .filter_map(|a| match a {
                                syn::GenericArgument::Type(t) => Some(t),
                                _ => None,
                            })
                            .collect();
                        if args.len() == 2 {
                            let key = rust_type_from_syn_at(args[0], aliases, depth);
                            let value = rust_type_from_syn_at(args[1], aliases, depth);
                            if is_composable(&key) && is_composable(&value) {
                                return RustType::BTreeMap(Box::new(key), Box::new(value));
                            }
                        }
                    }
                    RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
                }
                // The "owning wrapper" composition shape (2026-09-02,
                // TODO.md). Matched on the bare last segment, same as every
                // other `std` type this parser recognises by name (`String`,
                // `Duration`, ...) -- deliberately as vulnerable to a
                // same-named unrelated type as those already are.
                "Box" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
                        && let Some(syn::GenericArgument::Type(inner_ty)) = ab.args.first()
                    {
                        let inner = rust_type_from_syn_at(inner_ty, aliases, depth);
                        if is_composable(&inner) {
                            return RustType::BoxT(Box::new(inner));
                        }
                    }
                    RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
                }
                // Not a type this module knows by name. Reported by the
                // *bare* name for a plain path, which does two things: the
                // user-type resolution below looks a type up by its bare
                // name, so `v: crate::Beta` used to resolve to nothing and
                // be refused as unbuildable while `v: Beta` worked
                // (2026-08-28, same family as the qualified-`impl` fixes);
                // and the diagnostic stops quoting `crate :: Beta` at the
                // reader, which is `to_token_stream`'s spacing rather than
                // anything anyone wrote. Ambiguity is still caught: a bare
                // name declared in more than one file resolves to
                // `Ambiguous` and is refused by name.
                // A plain path is kept whole, spelled the way it is
                // written -- `crate::Beta`, `depx::Thing` -- rather than as
                // `to_token_stream`'s `crate :: Beta`, which is nobody's
                // spelling and which no lookup could match. Keeping the
                // qualifiers (rather than trimming to the last segment, as
                // a first attempt at this did) is what lets the user-type
                // resolution below check that the path the user wrote and
                // the declaration it found are the same type: trimming
                // made `depx::Thing` resolve to a local `Thing` of the same
                // name and build the wrong type entirely.
                _ if tp.qself.is_none() && seg.arguments.is_empty() => RustType::Unsupported(
                    tp.path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::"),
                ),
                _ => {
                    RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
                }
            }
        }
        // A shared reference is looked through: what matters for building
        // an arbitrary value is the type behind the `&`, which the harness
        // owns and lends. A **mutable** reference is not, and the
        // difference is not cosmetic -- it is a value the function writes
        // back, which neither engine here can construct or observe (§5.4b
        // stops at `&T`/`&[T]`). Looking through it recorded a plain `u32`
        // for a `&mut u32`, and the generated harness then passed a shared
        // reference where a mutable one was wanted: a compile failure
        // inside Ply's own generated file, reported to the user as an
        // internal tool error. Named as unsupported, it is a fact Ply
        // reports instead (`V0505`).
        Type::Reference(r) if r.mutability.is_some() => RustType::Unsupported(format!(
            "&mut {}",
            rust_type_from_syn_at(&r.elem, aliases, depth).display_name()
        )),
        Type::Reference(r) => rust_type_from_syn_at(&r.elem, aliases, depth),
        // `[T]` -- unsized, so the only legal spelling for a parameter is
        // behind a reference (`&[T]`, looked through just above), but this
        // arm is reached exactly that way: `Type::Reference`'s own `elem`
        // is this bare `Type::Slice`. Added 2026-09-02 (TODO.md, "add
        // slices"): not handled as a shape at all before this task, so
        // `&[T]` fell straight through to `Unsupported` regardless of `T`.
        // Built the same way `&Vec<u8>` already is -- see `RustType::
        // Slice`'s own doc.
        Type::Slice(s) => {
            let elem = rust_type_from_syn_at(&s.elem, aliases, depth);
            if is_composable(&elem) {
                RustType::Slice(Box::new(elem))
            } else {
                RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
            }
        }
        // `(A, B, ...)` -- added 2026-09-02 alongside slices (TODO.md). A
        // 0-element tuple is `()`, ordinary Rust for "no data"; as a
        // *parameter* type it is legal but never produced by real code, so
        // it is simply built as an empty tuple rather than special-cased.
        Type::Tuple(t) => {
            let elems: Vec<RustType> = t
                .elems
                .iter()
                .map(|e| rust_type_from_syn_at(e, aliases, depth))
                .collect();
            if elems.iter().all(is_composable) {
                RustType::Tuple(elems)
            } else {
                RustType::Unsupported(normalise_type_source(&ty.to_token_stream().to_string()))
            }
        }
        // A parenthesized type (`(u32)`, redundant parens around one type,
        // never a tuple -- `syn` only produces `Type::Tuple` when there is
        // a trailing comma or more than one element) is transparent: what
        // matters is the type behind the parens, the same "looked through"
        // treatment already given to a reference.
        Type::Paren(p) => rust_type_from_syn_at(&p.elem, aliases, depth),
        other => RustType::Unsupported(normalise_type_source(&other.to_token_stream().to_string())),
    }
}

/// Composition (2026-09-02, TODO.md) removed every per-shape "is this
/// leaf/scalar/composite-constructible" gate a composing arm above (`Vec`,
/// `BTreeSet`, `BTreeMap`, `Box`, `Option`, `Result`, `[T; N]`, `&[T]`, a
/// tuple) used to check its own inner type against before deciding to wrap
/// it -- **parsing decides shape, never buildability**. Whether either
/// engine can actually *build* a value of a composed type is
/// `is_bounded_supported`/`is_fuzz_supported`'s question, asked once the
/// whole tree is assembled; baking a narrower answer into the parser one
/// constructor at a time is exactly the "each shape barred from nesting"
/// defect this task fixes.
///
/// Always wrapping (this function is deliberately, permanently `true`) is
/// not merely harmless, it is required: the one thing the old
/// collapse-to-one-opaque-string behaviour could never do, and preserving
/// structure here fixes, is a **bare identifier not yet resolved to a user
/// type at parse time**. `Vec<Doc>`, parsed before `Doc`'s own crate-wide
/// scan ever runs, must stay `Vec<Unsupported("Doc")>` so the later
/// enrichment pass (`enrich_contract_fn_user_types`) can still find and
/// resolve the nested `Doc` -- flattening it to `Unsupported("Vec<Doc>")`
/// the way every composing arm used to on any non-composable inner would
/// destroy that one string's structure irrecoverably, the exact defect
/// measured in this task's own probe (a list of a user struct, refused).
/// An inner type genuinely unresolvable at parsing time (an opaque path, a
/// trait object, a generic parameter) already comes back its own
/// `Unsupported`, and wrapping it changes nothing observable: both support
/// predicates already propagate `false` through any composite whose
/// element is `Unsupported`, and `display_name`'s own recursive composition
/// reproduces the identical text the old flat fallback did. Kept as a named
/// call at every composing site above, rather than deleted outright, so
/// this reasoning lives in one place every one of them can point a reader
/// to instead of repeating it nine times.
fn is_composable(_ty: &RustType) -> bool {
    true
}

/// Collects top-level `type X = T;` items from a parsed file.
pub fn alias_map(file: &syn::File) -> AliasMap {
    let mut out = AliasMap::new();
    for item in &file.items {
        if let syn::Item::Type(ty) = item
            && ty.generics.params.is_empty()
        {
            out.insert(ty.ident.to_string(), (*ty.ty).clone());
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: RustType,
    pub by_ref: bool,
}

/// A contracted function as discovered from source, plus enough of its
/// §5.4a contract expressions (as both AST and source text) to drive
/// harness codegen and `contract_rt` rendering.
#[derive(Debug, Clone)]
pub struct ContractFn {
    /// The function's own identifier (`legacy_rate`), with no module path.
    pub name: String,
    /// Where the function lives, spelled from the crate root
    /// (`rates::legacy_rate`). Equal to `name` for a function declared at
    /// the top level of `src/lib.rs`. Generated code must call the function
    /// by *this*, because the module Ply generates sits at the crate root
    /// and a bare name only reaches a top-level function.
    pub path: String,
    pub params: Vec<Param>,
    /// `#[ply::requires(expr)]`, if present: the raw boolean expression.
    pub requires: Option<(Expr, String)>,
    /// `#[ply::ensures(|result| expr)]`, if present: the closure (its single
    /// parameter is conventionally named `result`, matching Kani's own
    /// `kani::ensures` shape) plus its source text for diagnostics.
    pub ensures: Option<(ExprClosure, String)>,
    /// Every free-function call in the body, in source order (§5.5's D5
    /// split is decided from these, before any engine runs).
    pub calls: Vec<crate::callgraph::CallSite>,
    /// The whole item as tokens, contract attributes included -- what
    /// §5.2a hashes first when it records this claim's result. A token
    /// stream and not the raw text on purpose: reformatting a function or
    /// editing a comment above it changes nothing about what was proved,
    /// and re-running a four-minute proof for a reflowed line is how a
    /// record earns a reputation for being wrong.
    pub source: String,
    /// Exact source location for a function resolved from first-party source.
    /// Direct callers of [`build_contract_fn`] do not have file context and
    /// therefore leave this absent; resolver-backed discovery always sets it.
    pub source_span: Option<crate::diag::Span>,
    /// Whether `path` is `Type::method` rather than a free function (added
    /// 2026-08-27, method resolution) -- see `call_expr`/`import_path`,
    /// which are the only things that need to know.
    pub is_method: bool,
    /// The return type, classified the same way a parameter's is --
    /// `Self` (bare, or nested in `Option`/`Result`) is its own shape
    /// (`RustType::SelfType`), because Ply never has to *construct* one: a
    /// constructor's return value is produced by the call, never built by
    /// `kani::any()`/proptest, so `Self`-shaped output blocks nothing.
    /// Anything else Ply's parser does not recognise is `Unsupported`, the
    /// same as an unrecognised parameter type, and gates checking this fn
    /// the same way (`is_bounded_supported`/`is_fuzz_supported`) -- added
    /// so that a genuinely un-modelled return shape is refused by name
    /// before codegen runs, never silently attempted (adversarial review,
    /// 2026-08-27).
    pub return_type: RustType,
    /// `Some` exactly when this is a method taking `&self` whose receiver
    /// Ply itself built -- a constructor call plus a bounded sequence of
    /// the type's own other operations (docs/review-self-construction.md's
    /// "fourth option": constructor-only is this with an empty operation
    /// pool and a sequence that always lands on length 0). `None` for every
    /// free function and every receiverless associated function (a
    /// constructor's own harness has nothing to build a receiver *for*).
    /// Never set by field-by-field construction -- there is no other
    /// producer of this field, on purpose (see `find_receiver_plan`'s own
    /// doc: the one and only place a `ReceiverPlan` is built).
    pub receiver: Option<ReceiverPlan>,
    /// Every top-level `use` item's local name in the file this fn was
    /// discovered in, mapped to its raw path segments -- exactly
    /// [`use_aliases_in_file`]'s own return value for that file, carried
    /// here so `fuzz_gen`'s codegen can resolve a name the *contract text*
    /// refers to (docs/reach-measurement-2.md: a postcondition naming
    /// `Ordering` directly -- never a parameter, never the return type --
    /// failed with `error[E0433]: cannot find type Ordering in this
    /// scope`, because the generated harness only ever imports the checked
    /// fn's own path and the types [`crate::fuzz_gen`]'s parameter/receiver
    /// walk finds; nothing walks the contract text itself). Empty for a
    /// `ContractFn` built directly with no file context -- same convention
    /// as `source_span`.
    pub use_aliases: std::collections::BTreeMap<String, Vec<String>>,
}

impl ContractFn {
    /// A single Rust identifier derived from `path`, for naming generated
    /// items (`ply_proof_rates_legacy_rate`). Two functions of the same name
    /// in different modules must not collide into one generated harness, so
    /// the whole path goes into the identifier, not just the last segment.
    /// For a top-level function this is exactly `name`.
    pub fn ident(&self) -> String {
        self.path.replace("::", "_")
    }

    /// Can Ply's Kani codegen build this fn's harness at all? (§5.4b gate,
    /// widened 2026-08-27 to also require the *return* type check out --
    /// see `RustType::is_bounded_return_supported`'s own doc for why that
    /// is a different question from a parameter's, never the same check
    /// twice.)
    ///
    /// A receiver method is refused here regardless of its params/return
    /// (this task, 2026-08-27): the review that settled receiver
    /// construction scoped the sequence-of-operations approach to the
    /// sampling tier first ("affordable at length 1-2 and falls off a
    /// cliff after" on the exhaustive tier, unmeasured), so `bounded(k)` on
    /// a receiver method stays `unsupported` rather than attempting a Kani
    /// harness nobody has measured the cost of. `fuzz`/`test` are what
    /// `is_fuzz_supported` below governs, and are unaffected.
    pub fn is_bounded_supported(&self) -> bool {
        self.receiver.is_none()
            && self.params.iter().all(|p| p.ty.is_bounded_supported())
            && self.return_type.is_bounded_return_supported()
    }

    /// Can Ply's proptest codegen build this fn's harness? (M4 gate --
    /// strictly broader, see `RustType::is_fuzz_supported`; widened
    /// 2026-08-27 the same way `is_bounded_supported` was.)
    pub fn is_fuzz_supported(&self) -> bool {
        self.params.iter().all(|p| p.ty.is_fuzz_supported())
            && self.return_type.is_fuzz_return_supported()
    }

    /// Whether this fn carries any contract at all (`requires` and/or
    /// `ensures`) -- the shape-aware default routing (§5.4c) only applies
    /// a default check to a contracted fn; an uncontracted fn defaults to
    /// no checks ("none otherwise").
    pub fn has_contract(&self) -> bool {
        self.requires.is_some() || self.ensures.is_some()
    }

    pub fn has_vec_param(&self) -> bool {
        self.params.iter().any(|p| matches!(p.ty, RustType::VecU8))
    }

    /// Whether any parameter or the return type is `f32`/`f64` -- what
    /// gates the NaN/infinity disclosure diagnostic (`W0518`, verify.rs):
    /// only a run that actually sampled a float owes the reader that
    /// disclosure.
    pub fn has_float_shape(&self) -> bool {
        self.params
            .iter()
            .any(|p| matches!(p.ty, RustType::F32 | RustType::F64))
            || matches!(self.return_type, RustType::F32 | RustType::F64)
    }

    /// Whether any parameter or the return type is `String` -- the
    /// content/length disclosure this run owes the reader, mirroring
    /// `has_float_shape`'s exact shape. Not yet wired to a CLI diagnostic
    /// (see `RustType::String`'s own doc for why this pass stops here);
    /// built ready for that wiring.
    pub fn has_string_shape(&self) -> bool {
        self.params.iter().any(|p| matches!(p.ty, RustType::String))
            || matches!(self.return_type, RustType::String)
    }

    /// The type name of every parameter Ply builds by direct field/variant
    /// construction (`RustType::UserTypeFields`, rule 2) -- what gates
    /// `public_fields_assumed_diag`'s own disclosure (verify.rs): a fuzz
    /// run that actually used this route owes the reader the "no
    /// invariant" assumption it rested on, the same way a float/`String`
    /// run discloses its own sampling choice. Empty for a fn with no such
    /// parameter (every constructor-built one, `RustType::UserTypeCtor`,
    /// needs no such disclosure -- nothing about it is assumed).
    pub fn public_fields_param_type_names(&self) -> Vec<String> {
        self.params
            .iter()
            .filter_map(|p| match &p.ty {
                RustType::UserTypeFields(plan) => Some(plan.type_name.clone()),
                _ => None,
            })
            .collect()
    }

    /// Every `UserTypeFieldsPlan::skipped_constructor` note among this fn's
    /// own parameters, in parameter order (2026-08-28,
    /// docs/review-structs-enums.md finding 2): a type built by direct
    /// field construction may still have a constructor Ply found but could
    /// not use, and `verify`'s `public_fields_assumed_diag` (W0522) must say
    /// so rather than reading as though direct construction were the only
    /// route available. Empty when no such parameter carries one.
    pub fn skipped_constructor_notes(&self) -> Vec<String> {
        self.params
            .iter()
            .filter_map(|p| match &p.ty {
                RustType::UserTypeFields(plan) => plan.skipped_constructor.clone(),
                _ => None,
            })
            .collect()
    }

    /// The expression generated code calls this fn by (added 2026-08-27,
    /// method resolution): the bare final identifier for a free function
    /// (`legacy_rate`), unchanged from before methods existed -- or, for a
    /// method, the last two segments of `path` (`Bucket::new`), since the
    /// method itself is not something a `use` can import (`use
    /// crate::Bucket::new;` does not compile) and must be reached through
    /// its type instead. Paired with `import_path`, which brings exactly
    /// the right name into scope for whichever shape this is.
    pub fn call_expr(&self) -> String {
        if self.is_method {
            last_two_segments(&self.path)
        } else {
            self.name.clone()
        }
    }

    /// The path a generated harness `use`s so `call_expr()` resolves: the
    /// whole `path` for a free function (unchanged -- this is what brings
    /// its bare final segment into scope), or `path` minus its final
    /// segment for a method (bringing the *type* into scope, since the
    /// method is reached off of it, not imported directly).
    pub fn import_path(&self) -> String {
        if self.is_method {
            match self.path.rsplit_once("::") {
                Some((rest, _method)) => rest.to_string(),
                None => self.path.clone(),
            }
        } else {
            self.path.clone()
        }
    }
}

/// The last two `::`-separated segments of `path` (`Type::method`), or the
/// whole thing when it has fewer than two. `pub` so `fuzz_gen`'s direct
/// tests can pin this without duplicating the split logic.
pub fn last_two_segments(path: &str) -> String {
    let segs: Vec<&str> = path.split("::").collect();
    if segs.len() >= 2 {
        format!("{}::{}", segs[segs.len() - 2], segs[segs.len() - 1])
    } else {
        path.to_string()
    }
}

/// Builds a resolver over `src_path` alone, for callers that have no
/// long-lived one: the crate directory is inferred from the conventional
/// `<crate>/src/lib.rs` layout so file modules (`mod rates;`) still resolve.
pub fn resolver_for(src_path: &Path) -> Result<crate::callgraph::Resolver> {
    let src = std::fs::read_to_string(src_path)
        .with_context(|| format!("reading source at {}", src_path.display()))?;
    let crate_dir = src_path
        .parent()
        .and_then(|src_dir| src_dir.parent())
        .unwrap_or_else(|| Path::new("."));
    crate::callgraph::Resolver::new(&src, crate_dir, std::collections::BTreeMap::new())
        .with_context(|| format!("parsing source at {}", src_path.display()))
}

/// Every free function in this crate a claim could anchor to, as canonical
/// crate-root paths — the item index §5.2 wants behind `E0301`'s
/// "nearest-name suggestions".
///
/// Deliberately the *same* set [`discover_fn_with`] searches, not a wider or
/// a narrower one: a suggestion naming a function anchor resolution would
/// then fail to find would be worse than no suggestion. Until 2026-08-25
/// both sets stopped at the top level of `src/lib.rs`, which is why the
/// suggestion machinery agreed with the resolution machinery and both were
/// wrong about the same functions.
pub fn crate_fn_paths(src_path: &Path) -> Result<Vec<String>> {
    Ok(resolver_for(src_path)?.fn_index())
}

/// Resolves `fn_path` — written the way the `ply.yaml` claim writes it,
/// relative to its component's anchor — to the function it names, walking
/// `use` imports, inline `mod`s and file modules exactly as call
/// classification does (§5.5). One resolver answers both questions, so Ply
/// can no longer report a callee as unvouched-for and then refuse the claim
/// that would vouch for it.
pub fn discover_fn_with(
    resolver: &mut crate::callgraph::Resolver,
    fn_path: &str,
    src_path: &Path,
) -> Result<ContractFn> {
    resolve_anchor(resolver, fn_path, src_path).map_err(|e| match e {
        AnchorError::NotFound => anyhow::anyhow!(
            "E0301: could not find fn `{fn_path}` in {} or any module it declares (unresolvable \
             anchor)",
            src_path.display()
        ),
        other => anyhow::anyhow!("E0301: {other}"),
    })
}

/// Why an anchor did not resolve. Three different facts, and they take three
/// different sentences: a name that is nowhere (suggest the nearest one), a
/// function that is real but out of a crate-root harness's reach, and a
/// function Ply found and could not read the shape of.
#[derive(Debug)]
pub enum AnchorError {
    /// No such function, in `src/lib.rs` or any module it declares.
    NotFound,
    /// Found, but a private `fn` or a private `mod` between it and the crate
    /// root means the generated harness cannot name it.
    Private(String),
    /// Ply followed the path into first-party source and could not read it.
    Unreadable(String),
    /// Found and named, but its signature or its contract is a shape this
    /// slice does not support (`E0304`, `E0501`).
    Shape(anyhow::Error),
}

impl std::fmt::Display for AnchorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnchorError::NotFound => write!(f, "no such function in this crate"),
            AnchorError::Private(r) | AnchorError::Unreadable(r) => write!(f, "{r}"),
            AnchorError::Shape(e) => write!(f, "{e}"),
        }
    }
}

/// [`discover_fn_with`], with the reason kept as data rather than flattened
/// into a message — `check` needs to say which of the four things happened.
pub fn resolve_anchor(
    resolver: &mut crate::callgraph::Resolver,
    fn_path: &str,
    _src_path: &Path,
) -> std::result::Result<ContractFn, AnchorError> {
    match resolver.lookup_fn(fn_path) {
        crate::callgraph::Resolution::Found(found) => {
            if let Some(reason) = found.unnameable {
                return Err(AnchorError::Private(reason));
            }
            let source_span = found.source_span.clone();
            let mut contract = build_contract_fn(
                &found.item,
                &alias_map(&found.file),
                &found.canonical,
                found.is_method,
            )
            .map_err(AnchorError::Shape)?;
            contract.source_span = Some(source_span);
            contract.use_aliases = use_aliases_in_file(&found.file);
            Ok(contract)
        }
        crate::callgraph::Resolution::Opaque(reason) => Err(AnchorError::Unreadable(reason)),
        crate::callgraph::Resolution::NotFound => Err(AnchorError::NotFound),
        // A real item Ply found and will not check (a receiver, a generic
        // `impl` block, a trait method) or could not choose between (two
        // `impl` blocks defining the same name). Both are shape facts about
        // real code, the same family `AnchorError::Shape` already names --
        // `verify` gives each its own sharper diagnostic before it ever
        // reaches here (see `verify.rs`'s pre-check), so this arm is what
        // `cargo ply check` (and any other caller of `resolve_anchor`
        // directly) sees, and its existing "found, cannot read its shape"
        // wording is already true of all four reasons.
        crate::callgraph::Resolution::Refused(reason)
        | crate::callgraph::Resolution::Ambiguous(reason) => {
            Err(AnchorError::Shape(anyhow::anyhow!("{reason}")))
        }
    }
}

/// [`discover_fn_with`] for a caller with no resolver of its own.
pub fn discover_fn(src_path: &Path, fn_path: &str) -> Result<ContractFn> {
    let mut resolver = resolver_for(src_path)?;
    discover_fn_with(&mut resolver, fn_path, src_path)
}

/// `quote`'s `TokenStream::to_string()` inserts a space between every token
/// (`|result|` becomes `| result |`), which is faithful but fails the
/// newbie-bar bar for text a user reads in a diagnostic or a generated
/// test's doc comment. This is a deliberately narrow cosmetic cleanup for
/// the closure-pipe and leading-deref shapes this slice's own contracts
/// use -- not a general Rust pretty-printer.
pub fn tidy_contract_text(s: &str) -> String {
    s.replace("| ", "|")
        .replace(" |", "|")
        .replace("* ", "*")
        .replace(" . ", ".")
        .replace(" ()", "()")
        // `old(x)` is one construct, not a call to something called `old`
        // with a space in it -- and this text is what a diagnostic quotes
        // back at the reader as "the line you wrote".
        .replace("old (", "old(")
}

/// `pub` so D5's first branch (§5.5) can parse a same-crate callee's own
/// inline contract the same way any claimed fn's is parsed -- the one
/// difference between a fn `verify` checks directly and one reached only as
/// another claim's callee is which caller asked, never how the source is
/// read.
pub fn build_contract_fn(
    f: &ItemFn,
    aliases: &AliasMap,
    path: &str,
    is_method: bool,
) -> Result<ContractFn> {
    let name = f.sig.ident.to_string();
    let mut params = Vec::new();
    for arg in &f.sig.inputs {
        let FnArg::Typed(pt) = arg else {
            bail!("E0304: `self` parameters are not supported in this slice");
        };
        let pname = match &*pt.pat {
            Pat::Ident(pi) => pi.ident.to_string(),
            _ => bail!("E0304: unsupported parameter pattern (only plain identifiers)"),
        };
        // Only a *shared* reference is stripped here; a `&mut` keeps its
        // whole written type so `rust_type_from_syn` can refuse it by name.
        let (by_ref, inner_ty) = match &*pt.ty {
            Type::Reference(r) if r.mutability.is_none() => (true, r.elem.as_ref()),
            other => (false, other),
        };
        params.push(Param {
            name: pname,
            ty: rust_type_from_syn(inner_ty, aliases),
            by_ref,
        });
    }

    let mut requires = None;
    let mut ensures = None;
    for attr in &f.attrs {
        let segs: Vec<String> = attr
            .path()
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        if segs == ["ply", "requires"] {
            let expr: Expr = attr
                .parse_args()
                .context("E0501: could not parse #[ply::requires] as an expression")?;
            let text = tidy_contract_text(&expr.to_token_stream().to_string());
            requires = Some((expr, text));
        } else if segs == ["ply", "ensures"] {
            let closure: ExprClosure = attr
                .parse_args()
                .context("E0501: could not parse #[ply::ensures] as a `|result| expr` closure")?;
            let text = tidy_contract_text(&closure.to_token_stream().to_string());
            ensures = Some((closure, text));
        }
    }

    let return_type = return_rust_type_from_syn(&f.sig.output, aliases);

    Ok(ContractFn {
        name,
        path: path.to_string(),
        params,
        requires,
        ensures,
        calls: crate::callgraph::call_sites(f),
        source: f.to_token_stream().to_string(),
        source_span: None,
        is_method,
        return_type,
        receiver: None,
        use_aliases: std::collections::BTreeMap::new(),
    })
}

// == Receiver construction (docs/review-self-construction.md's "fourth
// option") =====================================================================
//
// A method taking `&self` is refused by `callgraph::Resolver` before it ever
// reaches `discover_fn_with` -- "cannot yet build a value of `Type` to call
// it on" (`callgraph::receiver_refusal_reason`). That refusal is correct for
// what `discover_fn_with`'s single shared resolver is for (§5.5: "one
// resolver answers both questions, so Ply can no longer have two ideas of
// where a function lives"), and this task does not touch it.
//
// What follows is a second, narrower, independent path a caller (`verify`)
// tries *after* that refusal, never instead of it: given the exact path a
// `ply.yaml` claim wrote (`module::Type::method`), read that one module file
// again, and see whether Ply's own machinery -- a constructor it already
// knows how to call, a bounded sequence of the type's own other `&self`
// operations -- can build a receiver to call the method on. Every value it
// produces is built by calling the type's own code, nothing else, so no
// invariant is assumed and nothing is taken on trust (the review's whole
// point). It fails closed: anything it cannot handle returns a
// [`ReceiverError`] naming why, and the caller falls back to the original
// refusal, unchanged.
//
// **Deliberately narrow, honestly so:**
// - only `&self` methods are attempted -- a `&mut self`/owned-`self` target
//   is refused exactly as before (`ReceiverError::MutableOrOwnedReceiver`):
//   Ply still has no way to state what such a call is supposed to change
//   about the receiver, which is a separate gap this task does not close;
// - only one module segment deep (`module::Type::method`, not
//   `a::b::Type::method`) -- deeper nesting is `ReceiverError::UnsupportedModulePath`,
//   a named limit rather than a silent wrong answer;
// - a constructor must return bare `Self` (`return_rust_type_from_syn`'s own
//   existing narrowing -- `Result<Self, E>`, a real shape in the rate
//   limiter fixture's `Quota::new`/`RefillRate::new`, is not recognised
//   here either, unchanged from every other consumer of that function);
// - the bounded sequence's *other* operations (beyond repeating the target
//   itself) may be `&self` or `&mut self`, of any parameter shape, so long
//   as every parameter is a type the fuzz tier can build a value for --
//   widened 2026-08-27 (docs/review-caveats.md N3, "the twelfth false
//   clean") from an earlier, narrower rule that required an operation's own
//   shape to match the checked method's exactly: that rule is exactly what
//   emptied the pool for an ordinary Rust type, whose `&mut self` mutator
//   almost never shares its read-only sibling's parameter list, so nothing
//   that could actually change the receiver's state ever qualified. Each
//   operation now draws its own arguments from its own strategy
//   (`fuzz_gen::receiver_pattern_and_strategy`), so a mixed-shape pool is
//   exactly what gets built. The checked method itself is always pooled
//   (operation zero), so the pool is never empty and constructor-only
//   (sequence length 0) is always the floor, never a total refusal --
//   though it is a floor with nothing above it whenever the impl block
//   the checked method lives in declares no other `&self`/`&mut self`
//   operation at all;
// - the checked method's own `#[ply::requires]`, and the constructor's own
//   if it declares one, gate every call Ply's generated harness makes to
//   them -- including the earlier ones inside the sequence, not only the
//   final checked call (2026-08-27, docs/review-caveats.md N2). A pooled
//   operation that is not the checked method itself carries no such gate:
//   only the checked method's and the constructor's own preconditions are
//   honoured, which is what this task was asked to fix.

/// How long a bounded operation sequence Ply will build before calling the
/// checked method -- named here once so the verdict-visibility disclosure
/// (`verify`'s `receiver_sequence_diag`, W0520) and the codegen that
/// actually bounds the generated `Vec` agree on the same number. Three,
/// not zero: constructor-only (`docs/review-self-construction.md`'s own
/// "reaches roughly three and a half of eleven stated invariants") is the
/// floor this dial already covers at length 0; the decisive gap the review
/// measured -- a bug behind a *second* or *third* call, unreachable from a
/// freshly built value -- is what a nonzero default exists to close. On the
/// sampling tier, unlike the exhaustive one, going from 0 to 3 costs nothing
/// combinatorial (proptest draws one random sequence per case, it does not
/// enumerate every one), which is why this task defaults it above zero on
/// the tier it is scoped to -- see the module doc for why the exhaustive
/// tier is left out of this pass entirely.
pub const MAX_RECEIVER_SEQUENCE_LEN: u32 = 3;

/// One other operation Ply may splice into the bounded sequence before the
/// checked call -- an inherent, non-generic, `&self`- or `&mut self`-taking
/// method living in the same `impl` block(s) this scan already read, found
/// alongside the checked method itself (which is always operation zero, see
/// `ReceiverPlan::operations`).
///
/// **Shape need not match the checked method's own** (2026-08-27,
/// docs/review-caveats.md N3, "the twelfth false clean"): the pool used to
/// require every operation's own non-`self` parameters to equal the checked
/// method's element-for-element (`params_match`, since removed), which is
/// exactly what emptied the pool for any ordinary type -- a `&mut self`
/// mutator almost never shares its read-only sibling's parameter list (an
/// `add(&mut self, k: u32)` beside a `get(&self)`, say), so nothing that
/// could actually change the receiver's state ever qualified, and every
/// generated case called the checked method on a value no earlier step had
/// touched. Each operation now generates its own arguments from its own
/// strategy (`fuzz_gen::receiver_pattern_and_strategy`), so a mixed-shape
/// pool is exactly what the sequence now builds.
/// One of the type's own `&self`/`&mut self` operations that was found
/// alongside the checked method but left out of [`ReceiverPlan::operations`]
/// because one of its own parameters is a type the fuzz tier cannot build a
/// value for (docs/review-structs-enums.md finding 1, "the fourteenth false
/// clean", 2026-08-28). Before this, such an operation simply vanished from
/// the pool -- correct on its own terms (codegen cannot call it without an
/// argument), but silent: the receiver-sequence disclosure (`verify`'s
/// `receiver_sequence_diag`, W0520) went on asserting "every value this run
/// saw was reachable by calling the type's own code, nothing else, so
/// nothing here was assumed", which is exactly backwards when the one
/// operation that changes the receiver's state is the one that got dropped.
/// Recording *what* was excluded and *why*, rather than only shrinking the
/// pool, is what lets that disclosure say something true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludedOperation {
    /// Same spelling convention as [`Operation::call_path`].
    pub call_path: String,
    /// A complete, newbie-bar sentence fragment naming the parameter and
    /// its type, e.g. "its `s: str` argument uses a type Ply cannot build a
    /// value for" -- built once here so every caller (today, only the one
    /// disclosure) renders the same wording rather than re-deriving it from
    /// the raw type.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// The full path as the checked method's own is spelled (`module::Type::name`
    /// or `Type::name`), so `last_two_segments` renders it the same way
    /// `ContractFn::call_expr` already does.
    pub call_path: String,
    /// Its own non-`self` parameters -- no longer required to match the
    /// checked method's own shape (see the struct doc). Every parameter
    /// here is confirmed buildable by the fuzz tier (`is_fuzz_supported`)
    /// before this operation is admitted to the pool at all
    /// (`scan_impls_for_receiver`), so codegen never has to refuse one
    /// mid-generation.
    pub params: Vec<Param>,
    /// Whether this operation's own receiver is `&mut self` (true) rather
    /// than `&self` (false) -- codegen borrows `__ply_receiver` accordingly
    /// (`&mut __ply_receiver` vs `&__ply_receiver`) and declares the
    /// receiver binding itself `mut` whenever any pooled operation needs it.
    /// The checked method itself (operation zero) is always `&self` --
    /// enforced before a `ReceiverPlan` is ever built (`MutableOrOwnedReceiver`)
    /// -- so this is `false` for `operations[0]` unconditionally.
    pub takes_mut_self: bool,
}

/// A receiver Ply built for a method, rather than one a user declared or one
/// filled in field-by-field (`docs/review-self-construction.md` rejects
/// both): a constructor call, plus a pool of the type's own `&self`/
/// `&mut self` operations a bounded random sequence may call before the
/// checked method runs. `operations[0]` is always the checked method itself
/// -- repeating it is what reaches the invariant the review's own worked
/// example needed a *second* call to find (a fresh value's first call is
/// always the easy branch); every other entry is a sibling operation found
/// in the same scan, of whatever shape its own signature has (see
/// `Operation`'s doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverPlan {
    /// The type this receiver is a value of, spelled the way a diagnostic
    /// should name it (bare, no module prefix -- `Bucket`, not
    /// `bucket::Bucket`).
    pub type_name: String,
    /// `type_name`, qualified from the crate root the way a `use` needs it
    /// (`bucket::Quota`, not bare `Quota`) -- added for struct/enum
    /// **parameters** (2026-08-27): when this plan sits *nested* inside
    /// another one's `ctor_params` (`Quota::new`'s own `refill: RefillRate`
    /// argument), `fuzz_gen::wrap_fn_harness_module` needs this to `use` the
    /// type at all, since nothing else names its module. For the *receiver's
    /// own* plan (built by `scan_impls_for_receiver`, never nested), this
    /// field is never read -- `ContractFn::import_path` already brings the
    /// receiver's own type into scope from the claim's own path -- so it is
    /// set to the bare `type_name` there rather than computed, harmlessly.
    pub import_path: String,
    /// The constructor's own full path, same spelling convention as
    /// [`Operation::call_path`].
    pub constructor: String,
    pub ctor_params: Vec<Param>,
    /// The constructor's own `#[ply::requires]`, if it declares one --
    /// honoured exactly like the checked method's own (2026-08-27,
    /// docs/review-caveats.md N2): a caller declaring `Gauge::new`'s own
    /// precondition wrote it so *every* caller of `new`, including Ply's
    /// generated harness, respects it. Before this, Ply built the receiver
    /// by calling the constructor with an unfiltered argument, so a
    /// constructor whose own contract forbids the value it was given
    /// panicked on entry, and that panic was reported as the checked
    /// method's own promise breaking -- a violation on code that cannot be
    /// false.
    pub ctor_requires: Option<Expr>,
    /// Whether `constructor` returns bare `Self` or `Result<Self, E>`
    /// (2026-08-28, docs/review-structs-enums.md finding 2) -- codegen reads
    /// this to decide whether the constructor call needs a rejecting `match`
    /// around it: `fuzz_gen::build_user_value_stmt` for a parameter's own
    /// plan, `fuzz_gen::receiver_preamble` for a receiver's (the latter
    /// widened 2026-08-31, defect 1, docs/reach-measurement-2.md -- before
    /// that fix this field was hardcoded to `Bare` for every receiver's
    /// plan, so a fallible constructor Ply had already used to build a
    /// *parameter* in the same run was reported as though it did not exist
    /// when the very same scan was asked to build a *receiver*).
    pub ctor_return: CtorReturn,
    pub operations: Vec<Operation>,
    /// Every other `&self`/`&mut self` operation this scan found in the same
    /// `impl` block(s) but could not admit to `operations` because one of
    /// its own parameters is not a type the fuzz tier can build
    /// (2026-08-28, docs/review-structs-enums.md finding 1). Named here,
    /// never merely dropped, so `receiver_sequence_diag` can say which
    /// operations this run never called and why -- the honesty fix for
    /// "the fourteenth false clean": a mutator this list names is a mutator
    /// this run's history cannot contain, however many cases it ran.
    pub excluded_operations: Vec<ExcludedOperation>,
    /// Every other buildable constructor this scan found for `type_name`
    /// but did not use to build this receiver -- named, never merely
    /// unused, so the coverage disclosure can say which states this run
    /// never started from (2026-08-28, docs/review-silent-narrowing.md
    /// finding 3, "the type has a second constructor Ply never calls"):
    /// `scan_impls_for_receiver` always builds a receiver by calling one
    /// fixed constructor (the first fully-buildable one found, in source
    /// order), so a type with more than one usable constructor has states
    /// -- everything reachable only by calling the others -- that no case
    /// this run generated ever started from, however many cases ran or how
    /// long the operation sequence. Always empty for a parameter's own
    /// plan (built by [`resolve_user_type`], never nested more than one
    /// constructor deep by design).
    pub other_constructors: Vec<String>,
    /// [`MAX_RECEIVER_SEQUENCE_LEN`], carried alongside the plan so a
    /// caller building the verdict-visibility disclosure never has to
    /// import the constant under a different name than what codegen used.
    pub max_sequence_len: u32,
    /// `Some` exactly when this plan was built through §5.4b's generator
    /// hook -- a `routes:` entry in `ply.yaml` naming a public function
    /// that returns this type, rather than a constructor this scan found on
    /// its own (rule 1, above). `None` for every other `ReceiverPlan`: a
    /// receiver's own plan (`scan_impls_for_receiver`), and a parameter
    /// built by rule 1's ordinary constructor scan.
    ///
    /// Carried here, rather than as a separate `RustType` variant, because
    /// nothing downstream needs to tell a route-built value apart from a
    /// constructor-built one *structurally* -- codegen calls both exactly
    /// the same way (this is still a `UserTypeCtor`). What a route adds is
    /// two things ordinary construction does not need: a mark naming the
    /// declaration so a reader knows the values came through a declared
    /// door (`docs/reach-measurement-2.md`'s `seeded` mark is the
    /// precedent), and the guard a route specifically needs and a found
    /// constructor does not -- nothing stops an author's function from
    /// ignoring its own inputs and returning the same value every time, so
    /// `verify` counts distinct values reaching a route-built parameter and
    /// says so when that count collapses (TODO.md, "the guard this cannot
    /// ship without").
    pub route: Option<RouteOrigin>,
}

/// What a route-built [`ReceiverPlan`] needs beyond an ordinary
/// constructor's: the exact declaration a reader would edit (so a mark or a
/// diagnostic can point at it), and whether the type can even be compared
/// for the degenerate-route guard above -- a type with no `#[derive(Debug)]`
/// cannot be printed or hashed by code Ply generates from outside the
/// crate, so the guard says "Ply could not tell how many distinct values
/// reached it" rather than guessing a count from nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteOrigin {
    /// The function path exactly as `ply.yaml`'s `routes:` named it
    /// (`open_handle`, `Token::via_route`) -- not resolved or canonicalised,
    /// so a diagnostic quotes back exactly what the author wrote.
    pub declared_as: String,
    /// Whether the type declares `#[derive(Debug)]` (by hand or by the
    /// derive macro -- both count, the same convention
    /// [`type_declares_default`] already uses for `Default`). Always
    /// `false` for a cross-crate route (`outside_crate`, below): there is
    /// no local source to read a derive off of, so "could not tell" is the
    /// honest answer, never a guess.
    pub debug_derivable: bool,
    /// Whether this route names a function §5.4b's cross-crate extension
    /// resolved (2026-09-02, defect 2) -- a path this crate's own source
    /// does not declare, whose own input types the author wrote out in
    /// `ply.yaml` (`routes: { T: path::to::fn(Type1, Type2) }`) because Ply
    /// has no source to infer them from. `false` for the original, local
    /// form (`routes: { T: local_fn }`), inferred from source exactly as
    /// before. The one thing this changes downstream: codegen's `use`
    /// import for this type must not be prefixed with the target crate's
    /// own name (`import_path` is already a full, absolute path -- `std::
    /// ffi::OsString`, not a bare name this crate declares).
    pub outside_crate: bool,
}

/// Why [`discover_method_with_receiver`] could not build a checkable method
/// out of a `&self` receiver -- named so a caller can decide whether to fall
/// back to the resolver's own original refusal (every variant here) or, one
/// day, offer a sharper fix. `Display` renders the plain sentence a
/// diagnostic quotes directly (newbie bar: names the type, says why, never
/// just "unsupported").
#[derive(Debug)]
pub enum ReceiverError {
    /// The exact method this task's file-based scan looked for is not in
    /// any non-generic, trait-free `impl` block it could find in the one
    /// module file `fn_path` names -- most likely it is behind a trait impl,
    /// a generic `impl` block, split across a file this narrow scan does
    /// not follow, or the claim path itself does not parse as
    /// `[module::]Type::method`. The caller's own resolver-level refusal
    /// already names the real reason in every one of those cases; this
    /// variant only ever means "try the original message".
    MethodNotFound,
    /// The method exists but takes `&mut self` (or owned `self`) rather than
    /// `&self` -- unchanged from before this task: Ply still has no way to
    /// state what such a call is supposed to change about the receiver, so
    /// building one would not be enough on its own (`callgraph`'s own
    /// `&mut self` reason already says this; this variant exists so the
    /// caller can fall back to it rather than silently doing nothing).
    MutableOrOwnedReceiver,
    /// This scan's own module-path convention (`module::Type::method`, one
    /// segment at most) does not reach this claim's path -- named rather
    /// than guessed at.
    UnsupportedModulePath,
    /// The file could not be read or did not parse as Rust at all.
    Unreadable,
    /// The type has no receiverless associated function anywhere in the
    /// scanned `impl` block(s) that returns bare `Self` -- there is nothing
    /// for Ply to build a receiver by calling.
    NoConstructor { type_name: String },
    /// Every candidate constructor found takes at least one parameter of a
    /// type Ply's checkers cannot build a value of -- named so a reader
    /// knows exactly which type blocked it, not merely that "a" constructor
    /// exists.
    UnsupportedConstructorParam {
        type_name: String,
        ctor_name: String,
        bad_type: String,
    },
    /// The only constructor candidate(s) found are private -- unusable from
    /// the fuzz harness Ply generates outside this crate, exactly like an
    /// unbuildable-argument one (2026-08-28, docs/review-structs-enums.md's
    /// "Also fix" list, "a private constructor"). Distinct from
    /// `UnsupportedConstructorParam` so the message never claims "private"
    /// is a *type*.
    PrivateConstructor {
        type_name: String,
        ctor_name: String,
    },
    /// The checked method's own parameter list contains a shape this scan's
    /// simple pattern reader does not recognise (a destructuring pattern
    /// rather than a plain identifier) -- the same limit `build_contract_fn`
    /// already names for a free function, reached here before that shared
    /// path is even called.
    UnsupportedParamPattern,
}

impl std::fmt::Display for ReceiverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReceiverError::MethodNotFound => write!(f, "the method was not found by this scan"),
            ReceiverError::MutableOrOwnedReceiver => {
                write!(f, "the method does not take a shared `&self` receiver")
            }
            ReceiverError::UnsupportedModulePath => {
                write!(
                    f,
                    "the claim's module path is deeper than this scan follows"
                )
            }
            ReceiverError::Unreadable => write!(f, "the module file could not be read or parsed"),
            ReceiverError::NoConstructor { type_name } => write!(
                f,
                "Ply cannot build a receiver for `{type_name}`: it has no associated function \
                 in the file it is declared in that builds a `{type_name}` value and takes only \
                 types Ply's checkers already know how to build -- constructing a receiver needs \
                 a constructor to call, and none was found"
            ),
            ReceiverError::UnsupportedConstructorParam {
                type_name,
                ctor_name,
                bad_type,
            } => write!(
                f,
                "Ply cannot build a receiver for `{type_name}`: its constructor `{ctor_name}` \
                 takes a parameter of type `{bad_type}`, which is a shape Ply's checkers do not \
                 build values for yet, so the constructor itself cannot be called"
            ),
            ReceiverError::PrivateConstructor {
                type_name,
                ctor_name,
            } => write!(
                f,
                "Ply cannot build a receiver for `{type_name}`: its only constructor \
                 (`{ctor_name}`) is private, and the fuzz harness Ply generates lives outside \
                 this crate, so it cannot call it"
            ),
            ReceiverError::UnsupportedParamPattern => write!(
                f,
                "the method's own parameter list uses a pattern this scan does not read (only \
                 plain identifiers are supported)"
            ),
        }
    }
}

/// The module-relative file `fn_path`'s own module segment names, one level
/// deep at most (see the module doc's own narrowing) -- `crate_dir/src/lib.rs`
/// for a bare `Type::method`, `crate_dir/src/{seg}.rs` or
/// `crate_dir/src/{seg}/mod.rs` for `seg::Type::method`.
fn receiver_module_file(crate_dir: &Path, module_segs: &[&str]) -> Result<PathBuf, ReceiverError> {
    let src_dir = crate_dir.join("src");
    if module_segs.is_empty() {
        return Ok(src_dir.join("lib.rs"));
    }
    if module_segs.len() > 1 {
        return Err(ReceiverError::UnsupportedModulePath);
    }
    let seg = module_segs[0];
    let flat = src_dir.join(format!("{seg}.rs"));
    if flat.is_file() {
        return Ok(flat);
    }
    let nested = src_dir.join(seg).join("mod.rs");
    if nested.is_file() {
        return Ok(nested);
    }
    Err(ReceiverError::UnsupportedModulePath)
}

/// `file_path`'s own module path, as a list of segments from the crate
/// root -- `crate_dir/src/lib.rs`/`main.rs` is `[]`, `crate_dir/src/a.rs`
/// or `crate_dir/src/a/mod.rs` is `["a"]`, `crate_dir/src/a/b.rs` is
/// `["a", "b"]`. Shared by [`qualified_type_path`] (which appends a type's
/// own bare name to this) and by the receiver scan's own qualified-`impl`
/// resolution ([`resolve_absolute_path`]), so the two can never derive a
/// module path two different ways.
fn file_module_segments(crate_dir: &Path, file_path: &Path) -> Vec<String> {
    let src_dir = crate_dir.join("src");
    let rel = file_path.strip_prefix(&src_dir).unwrap_or(file_path);
    let mut segs: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(last) = segs.last().cloned() {
        if last == "lib.rs" || last == "main.rs" || last == "mod.rs" {
            segs.pop();
        } else if let Some(stem) = last.strip_suffix(".rs") {
            let stem = stem.to_string();
            *segs.last_mut().unwrap() = stem;
        }
    }
    segs
}

/// Resolves a path's segments (an `impl` block's own `self_ty`, or a `use`
/// tree leaf's path -- both are ordinary item-position paths) to an
/// absolute, crate-root-relative sequence of segments, the same "uniform
/// paths" rule Rust 2018+ item paths already follow: `crate::`/`self::`/
/// `super::` are relative keywords resolved against `file_mod` (the
/// scanning file's own module path, from [`file_module_segments`]);
/// anything else is already absolute from the crate root (or names an
/// external crate this scan does not track, in which case treating it as
/// absolute-from-root is a harmless best effort -- it will simply fail to
/// match any in-crate canonical path and read as [`ImplMatch::Unconfirmed`],
/// never as a false [`ImplMatch::Confirmed`]).
///
/// `None` only for a `super::` chain that pops past the crate root (a
/// shape that does not compile in the real crate either, so this scan will
/// never see it fed a valid file, but a parser is not a compiler and must
/// not panic on one).
fn resolve_absolute_path(file_mod: &[String], segs: &[String]) -> Option<Vec<String>> {
    let mut base: Vec<String>;
    let mut rest = segs;
    match rest.first().map(|s| s.as_str()) {
        Some("crate") => {
            base = Vec::new();
            rest = &rest[1..];
        }
        Some("self") => {
            base = file_mod.to_vec();
            rest = &rest[1..];
        }
        Some("super") => {
            base = file_mod.to_vec();
            while rest.first().map(|s| s.as_str()) == Some("super") {
                if base.is_empty() {
                    return None;
                }
                base.pop();
                rest = &rest[1..];
            }
        }
        _ => base = Vec::new(),
    }
    base.extend(rest.iter().cloned());
    Some(base)
}

/// Every local binding a top-level `use` item introduces in `file`, mapped
/// to the raw path segments (exactly as written, `crate`/`self`/`super`
/// kept literal) it names -- `use crate::till::Till as T;` contributes
/// `"T" -> ["crate", "till", "Till"]`; a plain `use till::Till;` (no
/// rename) contributes `"Till" -> ["till", "Till"]`. Built once per file so
/// [`classify_impl_self_ty`] can resolve `impl T { .. }` back to whatever
/// `T` actually names, the one shape a bare identifier comparison against
/// `type_name` cannot see on its own (2026-08-28, coordinator review of
/// docs/review-silent-narrowing.md's own fix, "a `use ... as` alias").
/// Glob imports (`use a::b::*;`) name nothing specific and contribute
/// nothing.
fn use_aliases_in_file(file: &syn::File) -> std::collections::BTreeMap<String, Vec<String>> {
    fn walk(
        tree: &syn::UseTree,
        prefix: &mut Vec<String>,
        out: &mut std::collections::BTreeMap<String, Vec<String>>,
    ) {
        match tree {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                walk(&p.tree, prefix, out);
                prefix.pop();
            }
            syn::UseTree::Name(n) => {
                let mut full = prefix.clone();
                full.push(n.ident.to_string());
                out.insert(n.ident.to_string(), full);
            }
            syn::UseTree::Rename(r) => {
                let mut full = prefix.clone();
                full.push(r.ident.to_string());
                out.insert(r.rename.to_string(), full);
            }
            syn::UseTree::Glob(_) => {}
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    walk(item, prefix, out);
                }
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    for item in &file.items {
        if let syn::Item::Use(u) = item {
            let mut prefix = Vec::new();
            walk(&u.tree, &mut prefix, &mut out);
        }
    }
    out
}

/// Whether an `impl` block's own `self_ty` is the same type as `type_name`
/// (2026-08-28, coordinator review of docs/review-silent-narrowing.md's own
/// fix: cross-file widening caught the plain `impl Till` spelling and
/// missed every qualified one -- `impl super::Till`, `impl crate::Till`,
/// `impl self::Till`, and an `impl` reached only through a `use ... as`
/// alias, every one of them exactly what a real submodule-split crate
/// writes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImplMatch {
    /// The same type, and this scan is sure of it: either the ordinary
    /// bare-name spelling (`impl Till`), or a qualified/aliased spelling
    /// this scan resolved to the type's own canonical, unambiguous
    /// declaring module.
    Confirmed,
    /// This `impl` block's own self type ends in the same bare identifier
    /// as `type_name`, but this scan could not confirm the two name the
    /// same declared type -- the type's own declaration is ambiguous
    /// crate-wide, or this scan's own resolution of the qualified/aliased
    /// path did not land on the type's canonical module. Never silently
    /// folded into `Confirmed` (a wrong merge) or into `NotThisType` (a
    /// silent absence): named as an exclusion instead, by the caller.
    Unconfirmed,
    /// An ordinary `impl` block for a different type entirely (a different
    /// bare name at the end of the path) -- not this scan's concern.
    NotThisType,
}

/// A plain type path's segments joined with `::`, no extra whitespace --
/// `imp.self_ty.to_token_stream().to_string()` renders `super :: Till`
/// (spaced, `quote`'s own token-stream style), which is accurate but not a
/// sentence a newbie-bar message should quote. Falls back to the
/// token-stream rendering for any shape other than a plain path (should
/// not occur here: every caller already matched `Type::Path` first).
/// A field's type as the source spells it, for the reader of a drawing.
///
/// Deliberately not [`render_type_path`], which drops generic arguments
/// because its callers compare *identity* -- `Vec<A>` and `Vec<B>` are the
/// same path and a different type. A drawn row is the opposite question:
/// what is in the list is usually the only part of that line worth reading,
/// so this keeps every argument and only tidies the spacing `syn`'s token
/// stream puts around punctuation.
fn render_type_display(ty: &Type) -> String {
    let raw = ty.to_token_stream().to_string();
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // A space before punctuation is always the token stream's, never
            // the author's.
            ' ' if matches!(chars.peek(), Some(',' | ';' | '>' | ']' | ')')) => {}
            // `< >` and `( )` never want a space just inside them; `Vec <`
            // never wants one just before.
            '<' | '(' | '[' => {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push(c);
                while chars.peek() == Some(&' ') {
                    chars.next();
                }
            }
            // `:: ` and ` ::` are both the token stream's doing.
            ':' if chars.peek() == Some(&':') => {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push(':');
                out.push(':');
                chars.next();
                while chars.peek() == Some(&' ') {
                    chars.next();
                }
            }
            '&' => {
                out.push('&');
                while chars.peek() == Some(&' ') {
                    chars.next();
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn render_type_path(ty: &Type) -> String {
    if let Type::Path(tp) = ty {
        tp.path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    } else {
        ty.to_token_stream().to_string()
    }
}

/// Classifies one `impl` block's `self_ty` against `type_name`, using
/// `file_mod` (the scanning file's own module path) and `use_aliases` (that
/// file's own `use ... as` bindings) to resolve `super::`/`crate::`/
/// `self::` qualifiers and aliases the same way `rustc` would, and
/// `target_absolute` (the type's own canonical absolute path -- module
/// segments plus its bare name -- when [`scan_crate_type_locations`] found
/// it declared in exactly one file crate-wide, `None` when it is declared
/// nowhere this scan found or in more than one file) to decide whether a
/// qualified/aliased spelling really lands on the same declaration.
fn classify_impl_self_ty(
    self_ty: &Type,
    type_name: &str,
    file_mod: &[String],
    use_aliases: &std::collections::BTreeMap<String, Vec<String>>,
    target_absolute: Option<&[String]>,
) -> ImplMatch {
    let Type::Path(tp) = self_ty else {
        return ImplMatch::NotThisType;
    };
    if tp.qself.is_some() {
        return ImplMatch::NotThisType;
    }
    let Some(last_seg) = tp.path.segments.last() else {
        return ImplMatch::NotThisType;
    };
    if !last_seg.arguments.is_empty() {
        // A generic instantiation (`impl Till<u8>`, or `impl mod::Wrapper<Till>`
        // whose *last* segment carries the args) -- a different question
        // from plain-name resolution, left entirely to the resolver's own
        // generic-`impl` handling, exactly as `impl_targets_type` already
        // left it.
        return ImplMatch::NotThisType;
    }

    let raw_segs: Vec<String> = tp
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();

    // The ordinary, overwhelmingly common shape: bare, one segment, the
    // same identifier as `type_name` -- always confirmed, exactly as
    // `impl_targets_type` already decided it, and checked first so it never
    // pays for alias/path resolution it does not need.
    if raw_segs.len() == 1 && raw_segs[0] == type_name {
        return ImplMatch::Confirmed;
    }

    // A single-segment self type that is *not* `type_name` itself can only
    // still be our type through a local `use ... as` alias -- resolve it to
    // whatever that alias's own path names before going further. A
    // multi-segment path is never itself an alias target (its first
    // segment is a module qualifier, not a local binding).
    let effective_segs: Vec<String> = if raw_segs.len() == 1 {
        match use_aliases.get(&raw_segs[0]) {
            Some(aliased) => aliased.clone(),
            None => raw_segs.clone(),
        }
    } else {
        raw_segs.clone()
    };

    if effective_segs.last().map(|s| s.as_str()) != Some(type_name) {
        return ImplMatch::NotThisType;
    }

    let Some(target_absolute) = target_absolute else {
        return ImplMatch::Unconfirmed;
    };
    match resolve_absolute_path(file_mod, &effective_segs) {
        Some(resolved) if resolved == target_absolute => ImplMatch::Confirmed,
        _ => ImplMatch::Unconfirmed,
    }
}

/// Every non-`self` argument of `inputs` as a [`Param`], the same shape
/// `build_contract_fn` already reads for a free function's whole parameter
/// list -- `None` on the one pattern shape it refuses (anything but a plain
/// identifier), so a caller can tell "no parameters" (`Some(vec![])`) apart
/// from "a shape this reader does not understand".
fn params_from_inputs<'a>(
    inputs: impl Iterator<Item = &'a FnArg>,
    aliases: &AliasMap,
) -> Option<Vec<Param>> {
    let mut out = Vec::new();
    for arg in inputs {
        let FnArg::Typed(pt) = arg else {
            return None;
        };
        let pname = match &*pt.pat {
            Pat::Ident(pi) => pi.ident.to_string(),
            _ => return None,
        };
        let (by_ref, inner_ty) = match &*pt.ty {
            Type::Reference(r) if r.mutability.is_none() => (true, r.elem.as_ref()),
            other => (false, other),
        };
        out.push(Param {
            name: pname,
            ty: rust_type_from_syn(inner_ty, aliases),
            by_ref,
        });
    }
    Some(out)
}

/// `m`'s own signature with its receiver argument removed, converted to a
/// plain `syn::ItemFn` -- lets [`discover_method_with_receiver`] hand the
/// checked method straight to `build_contract_fn` and get every bit of its
/// existing contract/call/source-hash handling for free, instead of
/// duplicating it. Mirrors `callgraph::impl_fn_to_item_fn`'s own conversion
/// (a private helper of that file, which this task's scope does not touch)
/// plus the one extra step that function never needed: dropping the
/// receiver `build_contract_fn` would otherwise bail out on by name
/// (E0304).
fn strip_receiver_to_item_fn(m: &syn::ImplItemFn) -> ItemFn {
    let mut sig = m.sig.clone();
    sig.inputs = sig.inputs.iter().skip(1).cloned().collect();
    ItemFn {
        attrs: m.attrs.clone(),
        vis: m.vis.clone(),
        sig,
        block: Box::new(m.block.clone()),
    }
}

/// Same `#[ply::requires]` extraction `build_contract_fn` performs for the
/// checked function itself, factored out so a constructor found by
/// `scan_impls_for_receiver` can be held to its own declared precondition
/// too (2026-08-27, docs/review-caveats.md N2): a receiverless associated
/// function never goes through `build_contract_fn` at all -- it is not
/// itself a claim, only a value `discover_method_with_receiver` calls on
/// the checked method's behalf -- so without this, its own
/// `#[ply::requires]`, if it declares one, would be silently ignored by
/// the one path that ever calls it: Ply's own generated receiver.
fn extract_requires_expr(attrs: &[syn::Attribute]) -> Result<Option<Expr>> {
    for attr in attrs {
        let segs: Vec<String> = attr
            .path()
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        if segs == ["ply", "requires"] {
            let expr: Expr = attr
                .parse_args()
                .context("E0501: could not parse #[ply::requires] as an expression")?;
            return Ok(Some(expr));
        }
    }
    Ok(None)
}

/// One candidate constructor found while scanning: its call path, its own
/// parameters, the first unbuildable-parameter type it has (if any), its
/// own declared `#[ply::requires]` (if any), and which of the two
/// constructor-return shapes it has ([`CtorReturn`], added 2026-08-31 --
/// see `scan_file_for_receiver`'s own doc for why) -- see
/// `scan_impls_for_receiver`.
type CtorCandidate = (String, Vec<Param>, Option<String>, Option<Expr>, CtorReturn);

/// One constructor candidate found by [`scan_ctor_candidates`]/
/// [`scan_ctor_candidates_crate_wide`] for a **parameter's** own type (never
/// a receiver's -- see [`CtorCandidate`] for that): its call path, its own
/// raw (not yet recursively resolved) parameters, its own declared
/// `#[ply::requires]` (if any), which of the two constructor-return shapes
/// it has ([`CtorReturn`]), and whether it is `pub`.
type ParamCtorCandidate = (String, Vec<Param>, Option<Expr>, CtorReturn, bool);

/// Every `.rs` file under `crate_dir/src`, recursing into subdirectories --
/// the same walk [`scan_crate_type_locations`]/`scan_ctor_candidates_crate_wide`
/// already do for a struct/enum *parameter*'s own lookup, factored out here
/// so the receiver scan's own crate-wide widening (2026-08-28,
/// docs/review-silent-narrowing.md finding 1, "the type's only mutator
/// lives in a second file") can reuse the same walk rather than re-deriving
/// it a third time. Order does not matter to any caller: every consumer
/// either merges into an order-independent structure or sorts its own
/// output afterward.
fn crate_source_files(crate_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![crate_dir.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// One file's own contribution to a receiver's pool, before the candidates
/// found across every file the crate has are merged and a constructor is
/// chosen ([`scan_impls_for_receiver`]).
#[derive(Default)]
struct FileReceiverScan {
    target: Option<syn::ImplItemFn>,
    ctor_candidates: Vec<CtorCandidate>,
    other_ops: Vec<Operation>,
    excluded_ops: Vec<ExcludedOperation>,
}

/// Scans every `impl {type_name} { .. }` block in `file` for: the checked
/// method itself (must exist, must take `&self`), a constructor candidate
/// (a receiverless associated function returning bare `Self`), and every
/// other `&self`/`&mut self` operation whose own parameters are all types
/// the fuzz tier can build (no longer required to match the checked
/// method's own shape -- see `Operation`'s doc, docs/review-caveats.md N3).
/// Called once per file [`scan_impls_for_receiver`] visits; see the module
/// doc for the honesty conditions this narrows on purpose.
///
/// **A `trait` implementation's own `&self`/`&mut self` methods are
/// recorded as excluded, not skipped silently** (2026-08-28,
/// docs/review-silent-narrowing.md finding 2, "the type's only mutator is a
/// trait method"): this scan does not resolve which trait a call needs or
/// attempt a trait-qualified call, and that is a real, permanent limit
/// (calling through a trait needs the trait itself in scope and, for a
/// generic trait, a chosen instantiation -- real work, not attempted here),
/// so a mutating trait method is out of scope rather than absent from the
/// pool by mistake. A trait impl's *receiverless* associated functions
/// (its own constructor-shaped methods, `impl Default for Foo`'s `default`
/// among them) are left untouched by this fix -- still silently invisible,
/// exactly as before this task: recognising a trait method as a receiver
/// constructor candidate is separate, adjacent scope this task's three
/// reproductions never needed (docs/review-silent-narrowing.md's own
/// finding 2 is about a *mutator*, not a constructor).
fn scan_file_for_receiver(
    file: &syn::File,
    aliases: &AliasMap,
    type_name: &str,
    method_name: &str,
    file_mod: &[String],
    target_absolute: Option<&[String]>,
) -> std::result::Result<FileReceiverScan, ReceiverError> {
    let mut out = FileReceiverScan::default();
    let use_aliases = use_aliases_in_file(file);

    for item in &file.items {
        let syn::Item::Impl(imp) = item else {
            continue;
        };
        let match_kind = classify_impl_self_ty(
            &imp.self_ty,
            type_name,
            file_mod,
            &use_aliases,
            target_absolute,
        );
        if match_kind == ImplMatch::NotThisType {
            continue;
        }
        if match_kind == ImplMatch::Unconfirmed {
            // Ends in the same bare name as `type_name`, reached through a
            // qualifier or alias this scan could not resolve to the type's
            // own canonical module (2026-08-28, coordinator review of
            // docs/review-silent-narrowing.md's own fix) -- named as an
            // exclusion by the exact spelling written, never silently
            // folded into the pool (a wrong merge) or silently dropped (a
            // false claim of completeness), the same discipline the trait
            // branch below already applies to a different reason.
            let self_ty_src = render_type_path(&imp.self_ty);
            for impl_item in &imp.items {
                let syn::ImplItem::Fn(m) = impl_item else {
                    continue;
                };
                let has_ref_receiver = matches!(
                    m.sig.inputs.first(),
                    Some(FnArg::Receiver(r)) if r.reference.is_some()
                );
                if has_ref_receiver {
                    out.excluded_ops.push(ExcludedOperation {
                        call_path: format!("{type_name}::{}", m.sig.ident),
                        reason: format!(
                            "it is defined in `impl {self_ty_src}`, and this scan could not \
                             confirm that `{self_ty_src}` names the same `{type_name}` this \
                             receiver was built for"
                        ),
                    });
                }
            }
            continue;
        }
        if imp.trait_.is_some() {
            for impl_item in &imp.items {
                let syn::ImplItem::Fn(m) = impl_item else {
                    continue;
                };
                let has_ref_receiver = matches!(
                    m.sig.inputs.first(),
                    Some(FnArg::Receiver(r)) if r.reference.is_some()
                );
                if has_ref_receiver {
                    out.excluded_ops.push(ExcludedOperation {
                        call_path: format!("{type_name}::{}", m.sig.ident),
                        reason: "it is defined in a `trait` implementation, and this scan \
                                 only calls a type's own inherent methods, never a trait's"
                            .to_string(),
                    });
                }
            }
            continue;
        }
        if !imp.generics.params.is_empty() {
            continue;
        }
        for impl_item in &imp.items {
            let syn::ImplItem::Fn(m) = impl_item else {
                continue;
            };
            let is_receiverless = !matches!(m.sig.inputs.first(), Some(FnArg::Receiver(_)));
            if m.sig.ident == method_name && !is_receiverless {
                out.target = Some(m.clone());
                continue;
            }
            if is_receiverless {
                // A candidate constructor: bare `Self`, the type's own name
                // written out (`-> Quota`, `-> super::Quota`), or either of
                // those wrapped in `Result<_, E>` -- exactly
                // [`ctor_return_kind`]'s own answer, the same function
                // [`scan_ctor_candidates`] (the *parameter* path's own
                // scan) already calls. Before 2026-08-31 this scan asked its
                // own, narrower question here (bare-`Self`-or-name only),
                // so the very same constructor Ply had just used to build a
                // *parameter* in the same run (`A::new(v) -> Result<A,
                // Bad>`) was reported as not existing when the receiver
                // scan asked -- a false sentence, found pointing Ply at
                // `semver` (docs/reach-measurement-2.md). One classifier,
                // shared by both scans, is what makes that impossible now:
                // there is only one answer to "is this a constructor",
                // not a second one for receivers to disagree with.
                let Some(ctor_return) = ctor_return_kind(
                    &m.sig.output,
                    aliases,
                    type_name,
                    file_mod,
                    &use_aliases,
                    target_absolute,
                ) else {
                    continue;
                };
                let Some(params) = params_from_inputs(m.sig.inputs.iter(), aliases) else {
                    continue;
                };
                // A private constructor is unusable from the fuzz harness
                // Ply generates outside this crate, exactly like an
                // unbuildable-argument one -- checked first so a private
                // constructor never wins over a public one further down
                // this scan (2026-08-28, docs/review-structs-enums.md's
                // "Also fix" list, "a private constructor": "I confirmed the
                // same blindness on the object-construction path, so this is
                // one shared gap, not two").
                let bad = if !is_pub(&m.vis) {
                    Some("private".to_string())
                } else {
                    params
                        .iter()
                        .find(|p| !p.ty.is_fuzz_supported())
                        .map(|p| p.ty.display_name())
                };
                let path = format!("{type_name}::{}", m.sig.ident);
                // A constructor's own `#[ply::requires]`, if it declares
                // one, travels with the candidate so the chosen
                // constructor's precondition can gate the arguments Ply
                // generates for it (2026-08-27, docs/review-caveats.md N2).
                let ctor_requires = extract_requires_expr(&m.attrs)
                    .map_err(|_| ReceiverError::UnsupportedParamPattern)?;
                out.ctor_candidates
                    .push((path, params, bad, ctor_requires, ctor_return));
            } else if let Some(syn::FnArg::Receiver(r)) = m.sig.inputs.first()
                && r.reference.is_some()
                && let Some(params) = params_from_inputs(m.sig.inputs.iter().skip(1), aliases)
            {
                // A candidate sequence operation: `&self` *or* `&mut self`
                // (2026-08-27, docs/review-caveats.md N3 -- the ordinary
                // way a Rust type changes state), any shape, so long as
                // every one of its own parameters is a type the fuzz tier
                // can build a value for. An unbuildable-type operation is
                // left out of the pool rather than guessed at, the same
                // discipline the constructor candidates already use just
                // above -- but never *silently*: an operation excluded here
                // is exactly the operation this type's state depends on
                // changing (docs/review-structs-enums.md finding 1, "the
                // fourteenth false clean", 2026-08-28), so it is recorded
                // by name and reason rather than only vanishing from
                // `other_ops`.
                //
                // Whether an argument is buildable is decided by the caller
                // (`scan_impls_for_receiver`), not here. This scan is purely
                // syntactic: it has no crate-wide type index, so a bare
                // `Flag` still reads `Unsupported` at this point whether or
                // not the crate declares a perfectly buildable enum by that
                // name. Judging here excluded operations for types that were
                // never unbuildable, only unresolved -- the same defect the
                // constructor candidates above already deferred out of this
                // function for the same reason, and the one the false-green
                // work ended on (TODO.md, 2026-09-03): a mutator taking a
                // plain enum was dropped from the sequence pool while the
                // identical enum built fine as an ordinary parameter.
                out.other_ops.push(Operation {
                    call_path: format!("{type_name}::{}", m.sig.ident),
                    params,
                    takes_mut_self: r.mutability.is_some(),
                });
            }
        }
    }
    Ok(out)
}

/// Builds a receiver's pool by merging [`scan_file_for_receiver`] over the
/// method's declaring file plus every other `.rs` file under the crate's
/// `src/` (2026-08-28, docs/review-silent-narrowing.md finding 1): an
/// ordinary second `impl {type_name} { .. }` block elsewhere in the crate
/// is completely normal Rust, and this scan used to read the declaring file
/// only, so a type whose only mutating method lived in a different file had
/// an operation pool of nothing but reads -- and reported the receiver
/// disclosure's strongest sentence ("nothing here was assumed") over a
/// history that never had the chance to call the one operation that
/// mattered. The crate-wide walk this reuses
/// ([`crate_source_files`]) already exists from the struct/enum parameter
/// work, so this is the same inventory, not a new one.
fn scan_impls_for_receiver(
    file: &syn::File,
    aliases: &AliasMap,
    type_name: &str,
    method_name: &str,
    crate_dir: &Path,
    declaring_file: &Path,
    routes: &RouteTable,
) -> std::result::Result<(syn::ImplItemFn, ReceiverPlan), ReceiverError> {
    let mut target: Option<syn::ImplItemFn> = None;
    let mut ctor_candidates: Vec<CtorCandidate> = Vec::new();
    let mut other_ops: Vec<Operation> = Vec::new();
    let mut excluded_ops: Vec<ExcludedOperation> = Vec::new();

    // Computed once, up front, and reused twice below: first (here) to
    // resolve a qualified/aliased `impl` self type against the type's own
    // canonical declaring module (2026-08-28, coordinator review, "which
    // other spellings resolve to the same type"), then again (further
    // down, unchanged from before this fix) to resolve a constructor
    // argument's own struct/enum type. One crate-wide scan, two consumers,
    // rather than two scans that could disagree.
    let locations = scan_crate_type_locations(crate_dir);
    let target_absolute: Option<Vec<String>> = match locations.get(type_name) {
        Some(Some(decl)) => {
            let mut segs = decl.module_segments(crate_dir);
            segs.push(type_name.to_string());
            Some(segs)
        }
        // Declared nowhere this scan found, or declared in more than one
        // file (ambiguous) -- either way, no qualified/aliased spelling can
        // be confirmed against it, so every one becomes `Unconfirmed`
        // rather than silently trusted.
        _ => None,
    };

    let mut merge = |scan: FileReceiverScan| {
        if target.is_none() {
            target = scan.target;
        }
        ctor_candidates.extend(scan.ctor_candidates);
        other_ops.extend(scan.other_ops);
        excluded_ops.extend(scan.excluded_ops);
    };
    let declaring_mod = file_module_segments(crate_dir, declaring_file);
    merge(scan_file_for_receiver(
        file,
        aliases,
        type_name,
        method_name,
        &declaring_mod,
        target_absolute.as_deref(),
    )?);

    for path in crate_source_files(crate_dir) {
        if path == declaring_file {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(other_file) = syn::parse_file(&src) else {
            continue;
        };
        let other_aliases = alias_map(&other_file);
        let other_mod = file_module_segments(crate_dir, &path);
        merge(scan_file_for_receiver(
            &other_file,
            &other_aliases,
            type_name,
            method_name,
            &other_mod,
            target_absolute.as_deref(),
        )?);
    }

    let Some(target) = target else {
        return Err(ReceiverError::MethodNotFound);
    };
    match target.sig.inputs.first() {
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none() => {}
        _ => return Err(ReceiverError::MutableOrOwnedReceiver),
    }
    let target_params = params_from_inputs(target.sig.inputs.iter().skip(1), aliases)
        .ok_or(ReceiverError::UnsupportedParamPattern)?;

    // Struct/enum parameters (2026-08-27): a constructor argument classified
    // `Unsupported` by the plain parser above may still be a struct/enum Ply
    // itself knows how to build (this is exactly `Quota::new`'s own
    // `refill: RefillRate` in the rate-limiter fixture) -- re-resolve every
    // candidate's own parameters, and the checked method's own, before
    // deciding what is buildable. Naive `is_fuzz_supported` on the
    // *unresolved* type would reject `Quota::new` here before this
    // recursion ever ran, which is exactly the bug this re-resolution
    // fixes: without it, a receiver whose constructor takes another
    // buildable user type is refused for a type that was never actually
    // unbuildable, just not resolved yet at scan time. `locations` is the
    // same crate-wide scan computed once, above, for `target_absolute`.
    let resolve_or_keep = |p: &Param| -> Param {
        match resolve_param_type(crate_dir, &locations, routes, &p.ty, 0) {
            Ok(ty) => Param { ty, ..(*p).clone() },
            Err(_) => p.clone(),
        }
    };
    // Every candidate operation gets the same re-resolution the constructor
    // candidates get just below, and only then is judged. An operation whose
    // arguments all resolve joins the sequence pool; one with an argument
    // that genuinely cannot be built is still named and its reason still
    // recorded -- the "fourteenth false clean" disclosure is unchanged, it
    // now just says it about operations that are really unbuildable rather
    // than ones nobody had looked up yet.
    let (other_ops, newly_excluded): (Vec<Operation>, Vec<ExcludedOperation>) = {
        let mut keep = Vec::new();
        let mut drop = Vec::new();
        for op in other_ops {
            let resolved: Vec<Param> = op.params.iter().map(resolve_or_keep).collect();
            match resolved.iter().find(|p| !p.ty.is_fuzz_supported()) {
                None => keep.push(Operation {
                    params: resolved,
                    ..op
                }),
                Some(bad) => drop.push(ExcludedOperation {
                    call_path: op.call_path.clone(),
                    reason: format!(
                        "its `{}: {}` argument uses a type Ply cannot build a value for",
                        bad.name,
                        bad.ty.display_name()
                    ),
                }),
            }
        }
        (keep, drop)
    };
    excluded_ops.extend(newly_excluded);
    let ctor_candidates: Vec<CtorCandidate> = ctor_candidates
        .into_iter()
        .map(|(path, params, _bad, req, ctor_return)| {
            let resolved: Vec<Param> = params.iter().map(resolve_or_keep).collect();
            let bad = resolved
                .iter()
                .find(|p| !p.ty.is_fuzz_supported())
                .map(|p| p.ty.display_name());
            (path, resolved, bad, req, ctor_return)
        })
        .collect();
    // `target_params` (the checked method's own non-`self` arguments) is
    // deliberately **not** run through `resolve_or_keep`: `receiver_preamble`'s
    // codegen (`fuzz_gen.rs`) calls the checked method with its own params'
    // *plain, already-bound* names, taken straight from the outer pattern --
    // it has no preamble-building path yet for one of those being a
    // struct/enum Ply itself constructs (only a constructor's own arguments,
    // and an ordinary top-level parameter, are wired). Leaving it
    // unresolved means such a param stays `Unsupported`, and the existing
    // `is_fuzz_supported` gate refuses the whole method honestly (naming
    // that parameter) rather than emitting a harness that cannot compile --
    // narrower than it could be, not broken.

    let buildable = ctor_candidates
        .iter()
        .find(|(_, _, bad, _, _)| bad.is_none());
    let (ctor_path, ctor_params, ctor_requires, ctor_return) = match buildable {
        Some((path, params, _, req, ctor_return)) => {
            (path.clone(), params.clone(), req.clone(), *ctor_return)
        }
        None => match ctor_candidates.first() {
            Some((path, _, Some(bad), _, _)) if bad == "private" => {
                return Err(ReceiverError::PrivateConstructor {
                    type_name: type_name.to_string(),
                    ctor_name: path.clone(),
                });
            }
            Some((path, _, Some(bad), _, _)) => {
                return Err(ReceiverError::UnsupportedConstructorParam {
                    type_name: type_name.to_string(),
                    ctor_name: path.clone(),
                    bad_type: bad.clone(),
                });
            }
            _ => {
                return Err(ReceiverError::NoConstructor {
                    type_name: type_name.to_string(),
                });
            }
        },
    };

    // Operation zero is always the checked method itself; every other
    // pooled operation found above is admitted as-is -- no shape match
    // against the target is required any more (2026-08-27, N3: that
    // requirement is exactly what emptied the pool for an ordinary
    // `&mut self`-mutating type).
    let mut operations = vec![Operation {
        call_path: format!("{type_name}::{method_name}"),
        params: target_params.clone(),
        takes_mut_self: false,
    }];
    operations.extend(other_ops);

    // Finding 3 (2026-08-28, docs/review-silent-narrowing.md, "the type has
    // a second constructor Ply never calls"): every other fully-buildable
    // constructor candidate this scan found -- across every file, now that
    // the scan above is crate-wide -- but did not choose, named rather than
    // merely discarded. This run still only ever starts a receiver's
    // history from one of them (`ctor_path`, picked deterministically as
    // the first fully-buildable one found); trying every constructor as a
    // separate starting point is real, further work this task's three
    // reproductions did not ask for (it would mean generating and running a
    // separate harness per constructor, or teaching one harness to choose
    // its own starting constructor the way it already chooses which pooled
    // operation to call next) -- named here so a reader knows which states
    // this run never started from, rather than a disclosure that goes
    // silent exactly where the first two findings' fix stops.
    let mut other_constructors: Vec<String> = ctor_candidates
        .iter()
        .filter(|(path, _, bad, _, _)| bad.is_none() && *path != ctor_path)
        .map(|(path, _, _, _, _)| path.clone())
        .collect();
    other_constructors.sort();
    other_constructors.dedup();

    let plan = ReceiverPlan {
        type_name: type_name.to_string(),
        // Never read for the receiver's own plan -- see the field's own
        // doc.
        import_path: type_name.to_string(),
        constructor: ctor_path,
        ctor_params,
        ctor_requires,
        // The chosen candidate's own answer from `ctor_return_kind`
        // (2026-08-31, defect 1, docs/reach-measurement-2.md) -- `Bare` for
        // a plain `-> Self`/`-> Type` constructor, `ResultSelf` for a
        // `Result`-wrapped one, exactly as the parameter path's own
        // `resolve_user_type` already reports for the same constructor.
        // Before this fix this field was hardcoded to `Bare` here, so a
        // fallible constructor Ply had just called to build a *parameter*
        // was reported as not existing at all when asked to build a
        // *receiver*.
        ctor_return,
        operations,
        excluded_operations: excluded_ops,
        other_constructors,
        max_sequence_len: MAX_RECEIVER_SEQUENCE_LEN,
        route: None,
    };
    Ok((target, plan))
}

/// The second, narrower path §"Receiver construction" above describes: given
/// the exact path a `ply.yaml` claim wrote, try to build a checkable
/// [`ContractFn`] for a `&self` method by finding the type's own constructor
/// and a pool of its own same-shape operations. Called only *after*
/// `callgraph::Resolver` has already refused the same path -- see the module
/// doc for why this never replaces that resolver, only follows it.
pub fn discover_method_with_receiver(
    crate_dir: &Path,
    fn_path: &str,
    routes: &RouteTable,
) -> std::result::Result<ContractFn, ReceiverError> {
    let segs: Vec<&str> = fn_path.split("::").collect();
    if segs.len() < 2 {
        return Err(ReceiverError::MethodNotFound);
    }
    let method_name = segs[segs.len() - 1];
    let type_name = segs[segs.len() - 2];
    let module_segs = &segs[..segs.len() - 2];

    let file_path = receiver_module_file(crate_dir, module_segs)?;
    let src = std::fs::read_to_string(&file_path).map_err(|_| ReceiverError::Unreadable)?;
    let file: syn::File = syn::parse_file(&src).map_err(|_| ReceiverError::Unreadable)?;
    let aliases = alias_map(&file);

    let (target, plan) = scan_impls_for_receiver(
        &file,
        &aliases,
        type_name,
        method_name,
        crate_dir,
        &file_path,
        routes,
    )?;

    let item_fn = strip_receiver_to_item_fn(&target);
    let mut cf = build_contract_fn(&item_fn, &aliases, fn_path, true)
        .map_err(|_| ReceiverError::UnsupportedParamPattern)?;
    cf.source_span = Some(crate::callgraph::source_span(
        crate_dir,
        &file_path,
        target.span(),
    ));
    cf.receiver = Some(plan);
    cf.use_aliases = use_aliases_in_file(&file);
    // Struct/enum parameters (2026-08-27): the checked method's own
    // parameters (not the receiver, not the constructor's own arguments --
    // both already resolved inside `scan_impls_for_receiver` above) get the
    // same chance any ordinary function's parameters do. Reasons for a
    // struct/enum found-but-refused are discarded here rather than
    // threaded through `ReceiverError` -- a receiver method's own
    // diagnostics are already keyed to the *receiver*, and widening that
    // shape to also carry a per-parameter refusal reason is more surface
    // than this task's own receiver-adjacent case needs; the generic
    // "type neither engine builds inputs for" message still names the
    // parameter and its type.
    let _ = enrich_contract_fn_user_types(&mut cf, crate_dir, routes);
    Ok(cf)
}

// == Struct/enum parameters (this task, 2026-08-27) =============================
//
// `docs/review-self-construction.md` settled how a value of the user's own
// type may be built; this task applies that same conclusion to an ordinary
// *parameter* (as opposed to `&self`, which the "Receiver construction"
// section above already covers). Literally the same rule, in order:
//
// 1. **Construct via the type's own constructor** where one exists that
//    takes buildable arguments, honouring its own precondition -- reusing
//    [`ReceiverPlan`] itself (with an empty operation pool: a parameter is
//    handed to the real function once, built, never a `self` further calls
//    run against). This is [`scan_ctor_candidates`] plus the recursive loop
//    in [`resolve_user_type`] below.
// 2. **Direct construction only when nothing is private** -- a struct whose
//    fields are all public, or an enum whose variants are all public and
//    carry only buildable public data. `#[non_exhaustive]` disqualifies it
//    (the fuzz harness is a separate crate depending on the target one, so
//    it is exactly the kind of "outside crate" `#[non_exhaustive]` blocks
//    from building a new variant/field literal) -- checked, not ignored,
//    per the review's own callout.
// 3. **Otherwise refuse by name**, naming the type and why
//    ([`UserTypeError::Refused`]).
//
// A struct/enum's own bare name is looked up across the *whole* crate
// ([`scan_crate_type_locations`]), never re-derived from the parameter's own
// module the way the receiver path's `Type::method` claim already names its
// module -- an ordinary parameter's type carries no such hint (it might be
// `use`d from anywhere), so the crate is scanned once for every top-level
// `struct`/`enum` declaration instead. A name declared in more than one file
// is refused as ambiguous rather than guessed at.
//
// **Recursion, bounded**: a constructor argument or a struct/enum field may
// itself be another buildable user type (`Quota::new`'s own `refill:
// RefillRate` argument, in the rate-limiter fixture, is exactly this shape)
// -- `resolve_user_type` calls itself, deeper each time, capped by
// [`MAX_USER_TYPE_DEPTH`] the same way alias-chasing is capped: a cycle
// cannot occur for an owned Rust value without indirection this parser does
// not follow anyway, but a parser is not a compiler and must not hang on
// unexpected input.
//
// **Deliberately narrow, honestly so:**
// - only a *named*-field struct, or an enum whose variants are all named-
//   field/unit, is recognised for rule 2 -- a tuple struct or a tuple
//   variant is refused by name rather than guessed at ([`UserTypeShape`]'s
//   own doc);
// - every variant must qualify for rule 2 to apply at all: one variant with
//   an unbuildable field refuses the *whole* enum, rather than silently
//   building only the variants Ply can reach -- the task's own question
//   ("is every variant reachable") is answered by refusing outright rather
//   than quietly narrowing, which would be exactly the kind of "false
//   clean" this project refuses on principle (§1);
// - a nested user type is only resolved as a *bare* top-level parameter/
//   field/argument, never inside `Option`/`Result`/`Vec`/`[T; N]` of one --
//   the same narrowing already applied to `NonZero`/`Duration`/`String`;
// - an operation Ply pools for a *receiver's* own bounded sequence
//   (`Operation::params`, the "Receiver construction" section above) is
//   deliberately **not** enriched here: `receiver_preamble`'s codegen has
//   no preamble-building path for an operation's own argument, only for the
//   checked call's and a constructor's, so a struct-typed operation
//   argument stays `Unsupported` and is filtered out of the pool by the
//   existing `is_fuzz_supported` gate -- a real function keeps checking
//   without that particular mutator in its sequence, rather than the
//   harness failing to compile.

/// Recursion bound for a chain of user types nested through constructor
/// arguments or struct/enum fields (`Quota::new`'s own `RefillRate`
/// argument is the real shape this exists for, at depth one) -- generous
/// enough for any realistic chain while still refusing to hang on
/// unexpected input, the same role [`MAX_ALIAS_DEPTH`] plays for a type
/// alias chain.
const MAX_USER_TYPE_DEPTH: usize = 6;

/// The most public fields direct field construction (rule 2) will build a
/// struct out of -- measured, not guessed (2026-08-28, docs/review-structs-
/// enums.md's "Also fix" list, "a struct with 13 or more fields"): the
/// generated strategy is a tuple of one value per field, and proptest's own
/// tuple `Strategy` impls (like the standard library's own tuple trait
/// impls) stop being generated past 12 elements, so a 13-field struct's
/// harness fails to compile with "the trait bound (…13 types…): Strategy is
/// not satisfied" -- raw compiler output about Ply's own generated code,
/// for a shape Ply had every field it needed to refuse by name instead.
const MAX_DIRECT_CONSTRUCTION_FIELDS: usize = 12;

/// Where Ply found a bare struct/enum name declared, scanning every `.rs`
/// file under a crate's `src/` directory (recursing into subdirectories,
/// following Ply's own file-per-module convention) -- keyed by bare name.
/// `None` records that the same bare name was found declared in more than
/// one file: ambiguous, refused by name (`UserTypeError::Ambiguous`) rather
/// than picking one arbitrarily.
/// Where one type is declared: the file, plus the inline `mod` blocks it
/// sits inside within that file.
///
/// The inline part is not decoration. `pub mod sub { pub struct Only; }`
/// written inside `lib.rs` is ordinary Rust, and an index that recorded
/// only the file put `Only` at the crate root -- so a parameter written
/// `sub::Only` did not match where Ply thought the type was, and a fully
/// public type with a public constructor was refused as if Ply had never
/// heard of it (2026-08-28, review finding: the inline-module blindness the
/// constructor scan had lost was still here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDecl {
    pub file: PathBuf,
    /// The inline `mod` names between the file's own module and the type,
    /// outermost first. Empty for a type at the top level of its file.
    pub inline_mods: Vec<String>,
    /// Whether every inline `mod` in that chain is `pub`. A type inside a
    /// private inline module is real, and unreachable from outside -- which
    /// is a different answer from "not found" and has to stay
    /// distinguishable.
    pub inline_mods_all_pub: bool,
}

impl TypeDecl {
    /// The type's module path from the crate root: the file's own module
    /// segments, then the inline `mod` names.
    pub fn module_segments(&self, crate_dir: &Path) -> Vec<String> {
        let mut segs = file_module_segments(crate_dir, &self.file);
        segs.extend(self.inline_mods.iter().cloned());
        segs
    }
}

/// One field of a type a component declares as its `state:` (§5.1).
///
/// `ty` is the parsed shape, so a drawing can tell a map from a list without
/// re-parsing the source, and `rendered` is the type spelled the way the
/// author wrote it, for the row's own label.
#[derive(Debug, Clone, PartialEq)]
pub struct StateField {
    pub name: String,
    pub ty: RustType,
    pub rendered: String,
}

/// Every field a type declares, in declaration order, whatever its
/// visibility -- `None` when the crate declares no type by that name.
///
/// Deliberately **not** [`resolve_user_type`]: that answers "can Ply build
/// one of these", and refuses a type with one private or unbuildable field.
/// This answers "what does this type hold", which is a different question
/// with a different honest answer. A private field is still something the
/// component holds, and a field of a type Ply has no strategy for is the
/// single most useful thing a reader can be shown -- it is usually the
/// reason that component's functions come back unsupported.
///
/// The distinction between "no type by that name" and "a type with no
/// fields" is kept, because a document naming a type the code does not
/// declare is the failure `state:` exists to catch (`A0414`), while a unit
/// struct really does hold nothing.
pub fn scan_type_fields(crate_dir: &Path, type_name: &str) -> Option<Vec<StateField>> {
    scan_type_fields_under(crate_dir, &[], type_name)
}

/// [`scan_type_fields`], restricted to types declared at or under one
/// module of the crate.
///
/// `under` is the component's anchor with the crate name removed:
/// `ply_core::visual` anchors at `["visual"]`, and a bare `ply_core` at the
/// empty slice, which accepts anything in the crate.
///
/// The restriction is the point rather than an optimisation. The spec says
/// a component's state is "resolved under its own anchor, never guessed
/// at", and `A0414` repeats that sentence to the user; a crate-wide scan
/// makes both untrue. The failure it lets through is one level subtler than
/// the one `state:` was built to catch -- the type really exists, so
/// nothing looks wrong, and only its *attribution* is false. Measured
/// before this existed: a component anchored at this crate's kernel
/// claimed a type declared in its diagnostics module and passed with exit
/// 0.
///
/// A type in a module *below* the anchor counts as under it: a component
/// anchored at `visual` may name a type declared in `visual::svg`, which is
/// still its own code.
pub fn scan_type_fields_under(
    crate_dir: &Path,
    under: &[String],
    type_name: &str,
) -> Option<Vec<StateField>> {
    let locations = scan_crate_type_locations(crate_dir);
    let decl = locations.get(type_name)?.as_ref()?;
    if !under.is_empty() {
        let declared_in = decl.module_segments(crate_dir);
        if !declared_in.starts_with(under) {
            return None;
        }
    }
    let src = std::fs::read_to_string(&decl.file).ok()?;
    let file: syn::File = syn::parse_file(&src).ok()?;
    let aliases = alias_map(&file);
    let items: &[syn::Item] = if decl.inline_mods.is_empty() {
        &file.items
    } else {
        items_at_inline_mods(&file.items, &decl.inline_mods)?
    };
    for item in items {
        let named = match item {
            syn::Item::Struct(st) if st.ident == type_name => match &st.fields {
                syn::Fields::Named(n) => Some(&n.named),
                // A tuple struct's fields have no names to write in `show:`,
                // so there is nothing a document could name. Reported as a
                // type with no nameable fields rather than as a missing
                // type: the type is real, it just cannot be shown this way.
                _ => return Some(Vec::new()),
            },
            _ => None,
        };
        if let Some(named) = named {
            return Some(
                named
                    .iter()
                    .filter_map(|f| {
                        let name = f.ident.as_ref()?.to_string();
                        Some(StateField {
                            name,
                            ty: rust_type_from_syn_at(&f.ty, &aliases, 0),
                            rendered: render_type_display(&f.ty),
                        })
                    })
                    .collect(),
            );
        }
    }
    None
}

/// Walks a non-empty chain of inline `mod` blocks, returning the items at
/// the end of it -- `None` if the chain does not exist. Callers handle the
/// empty chain themselves, since the answer there is the file's own items.
fn items_at_inline_mods<'a>(
    items: &'a [syn::Item],
    inline_mods: &[String],
) -> Option<&'a Vec<syn::Item>> {
    let mut current: Option<&Vec<syn::Item>> = None;
    let mut search: &[syn::Item] = items;
    for name in inline_mods {
        let found = search.iter().find_map(|it| match it {
            syn::Item::Mod(m) if m.ident == name.as_str() => m.content.as_ref().map(|(_, its)| its),
            _ => None,
        })?;
        current = Some(found);
        search = found;
    }
    current
}

pub type TypeLocations = std::collections::BTreeMap<String, Option<TypeDecl>>;

/// Builds [`TypeLocations`] for `crate_dir` -- a name -> file index, not a
/// resolver: a bare parameter type carries no hint about which module its
/// declaration lives in (unlike a `Type::method` claim's own path), so every
/// file under `src/` is read once. Cheap for the crate sizes this project
/// targets; a file that fails to parse is simply skipped (never a hang, and
/// a type declared only in an unparseable file is reported `NotFound`
/// downstream, the same honest answer as if it were not declared at all).
pub fn scan_crate_type_locations(crate_dir: &Path) -> TypeLocations {
    let mut out: TypeLocations = std::collections::BTreeMap::new();
    let mut stack = vec![crate_dir.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(file) = syn::parse_file(&src) else {
                continue;
            };
            // Inline modules count: the walk carries the `mod` chain down
            // with it, so a type inside `pub mod sub { .. }` is indexed at
            // `sub`, not at the crate root.
            let mut work: Vec<(&Vec<syn::Item>, Vec<String>, bool)> =
                vec![(&file.items, Vec::new(), true)];
            while let Some((items, mods, all_pub)) = work.pop() {
                for item in items {
                    if let syn::Item::Mod(m) = item
                        && let Some((_, inner)) = &m.content
                    {
                        let mut child = mods.clone();
                        child.push(m.ident.to_string());
                        work.push((inner, child, all_pub && is_pub(&m.vis)));
                        continue;
                    }
                    let name = match item {
                        syn::Item::Struct(s) => s.ident.to_string(),
                        syn::Item::Enum(e) => e.ident.to_string(),
                        _ => continue,
                    };
                    let decl = TypeDecl {
                        file: path.clone(),
                        inline_mods: mods.clone(),
                        inline_mods_all_pub: all_pub,
                    };
                    out.entry(name)
                        .and_modify(|slot| *slot = None)
                        .or_insert(Some(decl));
                }
            }
        }
    }
    out
}

/// Why Ply could not build a value of a user's own struct/enum type for a
/// **parameter** -- see the module doc above for the three-rule order this
/// answers. `Display` renders the plain sentence a diagnostic quotes
/// directly; `Refused` already carries a complete, type-naming sentence of
/// its own (built where the refusal actually happens, so it can be
/// specific), the other three are generic enough to render here.
#[derive(Debug, Clone)]
pub enum UserTypeError {
    /// Not a struct or enum this scan found declared anywhere under this
    /// crate's `src/` -- most often because the `Unsupported` type is not a
    /// user type at all (a generic parameter, a trait object, a type this
    /// task's scope does not reach), so this is not itself reported to a
    /// user: the generic "type neither engine builds inputs for" message
    /// already names it honestly.
    NotFound,
    /// More than one type shares this bare name across the crate.
    Ambiguous,
    /// The file declaring it could not be read or parsed as Rust.
    Unreadable,
    /// Found, real, and still refused -- rule 1 (constructor) and rule 2
    /// (direct construction) both failed, for the reason this sentence
    /// names.
    Refused(String),
}

impl std::fmt::Display for UserTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Reached both for a struct Ply genuinely could not find and
            // for a type that was never a struct at all -- `impl Into<u32>`,
            // a trait object, a generic parameter. The old wording ("not a
            // struct or enum found declared anywhere") sent the second
            // group looking for a missing declaration they never wrote.
            UserTypeError::NotFound => write!(
                f,
                "not a type Ply builds values of: neither one of the shapes its generators \
                 cover nor a struct or enum declared under this crate's `src/`"
            ),
            UserTypeError::Ambiguous => write!(
                f,
                "more than one type shares this bare name in this crate, and Ply does not guess \
                 which declaration a parameter means"
            ),
            UserTypeError::Unreadable => {
                write!(f, "the file declaring it could not be read or parsed")
            }
            UserTypeError::Refused(reason) => write!(f, "{reason}"),
        }
    }
}

/// A single Rust identifier and nothing else -- the only shape a bare
/// parameter type's `Unsupported` source text can be for this scan to even
/// attempt a lookup (`Vec < Foo >`, `& mut Bar`, `some :: Path` all fail
/// this and are left exactly as they were, honestly: they are real shapes
/// this task does not reach, not a struct/enum this scan could look up).
fn is_bare_ident(src: &str) -> bool {
    let mut chars = src.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn has_non_exhaustive(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("non_exhaustive"))
}

/// `fields` read as [`Param`]s, `None` for a tuple shape this reader does
/// not recognise (rule 2's own narrowing -- see `UserTypeShape`'s doc).
/// `Some(vec![])` for a unit struct/variant: real, buildable, zero fields.
fn named_fields_as_params(fields: &syn::Fields, aliases: &AliasMap) -> Option<Vec<Param>> {
    match fields {
        syn::Fields::Named(named) => {
            let mut out = Vec::new();
            for f in &named.named {
                out.push(Param {
                    name: f.ident.as_ref()?.to_string(),
                    ty: rust_type_from_syn(&f.ty, aliases),
                    by_ref: false,
                });
            }
            Some(out)
        }
        syn::Fields::Unit => Some(Vec::new()),
        syn::Fields::Unnamed(_) => None,
    }
}

/// Every field a struct/enum-variant's `fields` declares, all `pub` --
/// rule 2's own gate. Deliberately re-implemented here rather than reusing
/// `callgraph::Resolver::private_field_names`: that method answers a
/// different question (over a path *resolved* through a caller's own `use`
/// imports, from a resolver this module's scope does not touch), where this
/// scan already has the declaration itself open.
fn all_fields_public(fields: &syn::Fields) -> bool {
    fields
        .iter()
        .all(|f| matches!(f.vis, syn::Visibility::Public(_)))
}

/// Whether a visibility is plain `pub` -- never `pub(crate)`,
/// `pub(super)`, or bare (private): the fuzz harness Ply generates is a
/// separate crate (or, for the receiver path, calls back in from outside
/// the declaring module), so anything less than full `pub` is not callable
/// from it (2026-08-28, docs/review-structs-enums.md's "Also fix" list,
/// "a private constructor").
fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

/// Rule 1's own candidate scan, narrowed from [`scan_impls_for_receiver`]'s
/// (which also looks for a *target* method and an operation pool -- neither
/// applies to a parameter, which has no receiver to call further methods
/// on): every receiverless associated fn of `type_name` returning bare
/// `Self` *or* `Result<Self, E>` (widened 2026-08-28, see [`ctor_return_kind`]),
/// with its raw (not yet recursively resolved) parameters, its own
/// `#[ply::requires]`, which of the two return shapes it has, and whether
/// it is `pub` -- **unfiltered by publicness or buildability on purpose**:
/// unlike the older per-type scans, a candidate that turns out unusable
/// (a private fn, or a parameter that is itself an unresolved user type)
/// must not be discarded here, or `resolve_user_type` below could neither
/// resolve a nested candidate parameter (exactly the bug fixed in
/// `scan_impls_for_receiver` alongside this task, for `Quota::new`'s own
/// `RefillRate` argument) nor report a private one as *found but skipped*
/// rather than silently invisible (2026-08-28, "a private constructor").
fn scan_ctor_candidates(
    file: &syn::File,
    aliases: &AliasMap,
    type_name: &str,
    file_mod: &[String],
    target_absolute: Option<&[String]>,
) -> Vec<ParamCtorCandidate> {
    let mut out = Vec::new();
    let use_aliases = use_aliases_in_file(file);
    // Inline modules count. `mod inner { impl super::Alpha { .. } }` in the
    // same file is ordinary Rust, and walking only `file.items` skipped
    // every `impl` block written inside one -- so Ply again said a type
    // "has no constructor Ply can call" about a type with a public `new`
    // (2026-08-28, third cause of that same sentence). The module path is
    // carried down so `super::` inside the inline module resolves against
    // the module it is actually written in, not the file's own.
    let mut work: Vec<(&Vec<syn::Item>, Vec<String>)> = vec![(&file.items, file_mod.to_vec())];
    let mut flat: Vec<(&syn::Item, Vec<String>)> = Vec::new();
    while let Some((items, module)) = work.pop() {
        for item in items {
            if let syn::Item::Mod(m) = item
                && let Some((_, inner)) = &m.content
            {
                let mut child = module.clone();
                child.push(m.ident.to_string());
                work.push((inner, child));
            }
            flat.push((item, module.clone()));
        }
    }
    for (item, item_mod) in &flat {
        let file_mod: &[String] = item_mod;
        let syn::Item::Impl(imp) = item else {
            continue;
        };
        if imp.trait_.is_some() || !imp.generics.params.is_empty() {
            continue;
        }
        // The same resolution the receiver scan does (2026-08-28,
        // `classify_impl_self_ty`), rather than this path's older
        // bare-name-only test. A type declared in one module with its
        // `impl` block in a submodule has to write `impl super::T` -- there
        // is no other spelling available in that file -- and the older test
        // saw no constructor at all, so Ply told the user their type "has
        // no constructor Ply can call" about a type with a public `new`.
        // A false sentence, not merely a missing feature.
        //
        // `Unconfirmed` is deliberately not accepted: an `impl` ending in
        // the same bare name that this scan could not resolve to the type's
        // own canonical module might belong to a different type entirely,
        // and constructing the wrong type is worse than refusing.
        if classify_impl_self_ty(
            &imp.self_ty,
            type_name,
            file_mod,
            &use_aliases,
            target_absolute,
        ) != ImplMatch::Confirmed
        {
            continue;
        }
        for impl_item in &imp.items {
            let syn::ImplItem::Fn(m) = impl_item else {
                continue;
            };
            let is_receiverless = !matches!(m.sig.inputs.first(), Some(FnArg::Receiver(_)));
            if !is_receiverless {
                continue;
            }
            let Some(ctor_return) = ctor_return_kind(
                &m.sig.output,
                aliases,
                type_name,
                file_mod,
                &use_aliases,
                target_absolute,
            ) else {
                continue;
            };
            let Some(params) = params_from_inputs(m.sig.inputs.iter(), aliases) else {
                continue;
            };
            let Ok(ctor_requires) = extract_requires_expr(&m.attrs) else {
                continue;
            };
            out.push((
                format!("{type_name}::{}", m.sig.ident),
                params,
                ctor_requires,
                ctor_return,
                is_pub(&m.vis),
            ));
        }
    }
    out
}

/// [`scan_ctor_candidates`], run over every `.rs` file under `crate_dir`'s
/// `src/` rather than only `declaring_file` -- 2026-08-28,
/// docs/review-structs-enums.md finding 2, "the constructor lives in a
/// different file from the type": splitting a type's declaration from its
/// `impl` block across modules is an ordinary way to organise a Rust crate,
/// and the single-file scan used to miss any constructor placed anywhere
/// else, silently falling through to rule 2 (direct field construction) and
/// building the state the constructor exists to forbid. `declaring_file` is
/// scanned first, matching `scan_ctor_candidates`'s own source-order
/// preference within one file; every other file follows in a deterministic
/// (sorted-path) order, never re-scanning `declaring_file` itself.
fn scan_ctor_candidates_crate_wide(
    crate_dir: &Path,
    declaring_file: &Path,
    type_name: &str,
) -> Vec<ParamCtorCandidate> {
    let mut out = Vec::new();
    // The type's own canonical absolute path, so a qualified or aliased
    // `impl` spelling can be resolved back to it -- `None` when the type is
    // declared nowhere this scan found or in more than one file, in which
    // case no qualified spelling is confirmable and only the bare one
    // counts. Computed once for the whole walk, exactly as the receiver
    // scan does it.
    let locations = scan_crate_type_locations(crate_dir);
    let target_absolute: Option<Vec<String>> = match locations.get(type_name) {
        Some(Some(decl)) => {
            let mut segs = decl.module_segments(crate_dir);
            segs.push(type_name.to_string());
            Some(segs)
        }
        _ => None,
    };
    let parse_and_scan = |path: &Path, out: &mut Vec<ParamCtorCandidate>| {
        let Ok(src) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(file) = syn::parse_file(&src) else {
            return;
        };
        let aliases = alias_map(&file);
        let file_mod = file_module_segments(crate_dir, path);
        out.extend(scan_ctor_candidates(
            &file,
            &aliases,
            type_name,
            &file_mod,
            target_absolute.as_deref(),
        ));
    };
    parse_and_scan(declaring_file, &mut out);

    let mut files = Vec::new();
    let mut stack = vec![crate_dir.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    for path in files {
        if path == declaring_file {
            continue;
        }
        parse_and_scan(&path, &mut out);
    }
    out
}

/// Whether `file_path`'s type sits behind a private module somewhere
/// between it and the crate root -- `None` when every ancestor `mod`
/// declaration is `pub` (or there is none, because the type is declared at
/// the crate root), `Some(name)` naming the first (closest to the root)
/// private one otherwise (2026-08-28, docs/review-structs-enums.md's "Also
/// fix" list, "a private module"): `mod quota;` with no `pub` hides
/// `quota::Quota` from any harness Ply generates outside this crate, even
/// though `Quota` itself is `pub` and even though a `pub use` elsewhere may
/// re-export it under a different, public path -- this scan does not
/// follow re-exports, so it refuses rather than risk emitting a harness
/// that names an inaccessible path.
///
/// Segment derivation mirrors [`qualified_type_path`] exactly, so the two
/// never disagree about which module a file belongs to. **Fails open**
/// (`None`) whenever a `mod` declaration cannot be found or read -- an
/// inline `mod name { .. }` body (rather than a separate `name.rs`/
/// `name/mod.rs` file) is the one ordinary shape this narrow, one-file-per-
/// module scan does not follow (matching `scan_crate_type_locations`'s own
/// narrowing); refusing an ordinary crate on a scan gap would be a worse
/// mistake than missing this one refusal.
fn private_ancestor_module(crate_dir: &Path, file_path: &Path) -> Option<String> {
    let src_dir = crate_dir.join("src");
    let rel = file_path.strip_prefix(&src_dir).ok()?;
    let mut segs: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(last) = segs.last().cloned() {
        if last == "lib.rs" || last == "main.rs" || last == "mod.rs" {
            segs.pop();
        } else if let Some(stem) = last.strip_suffix(".rs") {
            *segs.last_mut().unwrap() = stem.to_string();
        }
    }
    for i in 0..segs.len() {
        let mod_name = &segs[i];
        let parent_file = if i == 0 {
            let lib = src_dir.join("lib.rs");
            let main = src_dir.join("main.rs");
            if lib.is_file() {
                lib
            } else if main.is_file() {
                main
            } else {
                return None;
            }
        } else {
            let parent_rel = segs[..i].join("/");
            let as_file = src_dir.join(format!("{parent_rel}.rs"));
            let as_mod_dir = src_dir.join(&parent_rel).join("mod.rs");
            if as_file.is_file() {
                as_file
            } else if as_mod_dir.is_file() {
                as_mod_dir
            } else {
                return None;
            }
        };
        let Ok(src) = std::fs::read_to_string(&parent_file) else {
            return None;
        };
        let Ok(file) = syn::parse_file(&src) else {
            return None;
        };
        let declared_vis = file.items.iter().find_map(|it| match it {
            syn::Item::Mod(m) if m.ident == *mod_name => Some(m.vis.clone()),
            _ => None,
        });
        match declared_vis {
            Some(vis) if !is_pub(&vis) => return Some(mod_name.clone()),
            Some(_) => {}
            None => return None,
        }
    }
    None
}

/// If `ty` is already something Ply knows how to build, returns it
/// unchanged; if it is `Unsupported` and its source text is a bare
/// identifier, tries to resolve it as a user-defined struct/enum
/// ([`resolve_user_type`]); anything else (a compound type expression Ply's
/// parser did not recognise -- `Vec<Foo>`, `&mut Bar`, a generic parameter)
/// is `NotFound` rather than guessed at.
fn resolve_param_type(
    crate_dir: &Path,
    locations: &TypeLocations,
    routes: &RouteTable,
    ty: &RustType,
    depth: usize,
) -> std::result::Result<RustType, UserTypeError> {
    let RustType::Unsupported(src) = ty else {
        return Ok(ty.clone());
    };
    if !is_bare_ident(src) {
        return Err(UserTypeError::NotFound);
    }
    resolve_user_type(crate_dir, locations, routes, src, depth)
}

/// The resolver at the centre of this section: try to build `RustType`
/// value-construction for the struct/enum named `type_name`, per the
/// module doc's three rules, in order. Recursive through
/// [`resolve_param_type`] for a constructor argument or a field that is
/// itself another user type, bounded by [`MAX_USER_TYPE_DEPTH`].
/// Whether the type has a `Default` -- hand-written as `impl Default for T`
/// or produced by `#[derive(Default)]`. Both count: the derive writes no
/// `fn default` into the source, so a scan reading only `impl` blocks would
/// find the hand-written one and miss the derived one, which is the same
/// false sentence one attribute away.
fn type_declares_default(file: &syn::File, type_name: &str, inline_mods: &[String]) -> bool {
    fn walk(items: &[syn::Item], type_name: &str) -> bool {
        for item in items {
            match item {
                syn::Item::Impl(imp) => {
                    let is_default = imp
                        .trait_
                        .as_ref()
                        .and_then(|(_, path, _)| path.segments.last())
                        .is_some_and(|seg| seg.ident == "Default");
                    if is_default
                        && let Type::Path(tp) = imp.self_ty.as_ref()
                        && tp
                            .path
                            .segments
                            .last()
                            .is_some_and(|s| s.ident == type_name)
                    {
                        return true;
                    }
                }
                syn::Item::Struct(s) if s.ident == type_name => {
                    if derives_default(&s.attrs) {
                        return true;
                    }
                }
                syn::Item::Enum(e) if e.ident == type_name => {
                    if derives_default(&e.attrs) {
                        return true;
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content
                        && walk(inner, type_name)
                    {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    let _ = inline_mods;
    walk(&file.items, type_name)
}

fn derives_default(attrs: &[syn::Attribute]) -> bool {
    derives(attrs, "Default")
}

/// Whether the type declares `#[derive(Debug)]` -- the degenerate-route
/// guard's own honesty condition (see [`RouteOrigin::debug_derivable`]): a
/// hand-written `impl Debug for T` would work too, but is not checked here
/// (unlike [`type_declares_default`]'s own hand-written-`impl` arm) because
/// a route is only ever declared for a type this scan already has the bare
/// declaration open for, and a derive is by far the ordinary case; widening
/// to a hand-written impl is possible future work, not attempted here.
fn derives_debug(attrs: &[syn::Attribute]) -> bool {
    derives(attrs, "Debug")
}

fn derives(attrs: &[syn::Attribute], trait_name: &str) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("derive")
            && a.parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
            )
            .map(|paths| paths.iter().any(|p| p.is_ident(trait_name)))
            .unwrap_or(false)
    })
}

/// The bare name a method's own path names its enclosing type by
/// (`Token::via_route` -> `Some("Token")`), or `None` for a path with no
/// second-to-last segment at all (a free function, or a bare name with no
/// module qualification). Used only to check a route function that returns
/// bare `Self` actually sits inside `impl <type_name>` -- the same question
/// [`return_names_this_type`] answers for rule 1's own constructor scan,
/// answered here from a resolved [`ContractFn::path`] instead of a raw
/// `syn::Type`, since [`discover_fn_with`] is what found this function.
fn enclosing_type_of(path: &str) -> Option<&str> {
    let segs: Vec<&str> = path.split("::").collect();
    (segs.len() >= 2).then(|| segs[segs.len() - 2])
}

/// Splits one `routes:` value into its function path and, when the author
/// wrote an explicit input-type list, those types verbatim -- §5.4b's
/// cross-crate extension (defect 2, 2026-09-02): `open_handle` (no parens)
/// is the original, local form, unchanged; `std::ffi::OsString::from(String)`
/// parses to `("std::ffi::OsString::from", Some(["String"]))`, because a
/// function outside this crate has no source here for Ply to read its
/// parameters from, so the author states them instead. A parenthesized,
/// empty list (`f()`) is `Some(vec![])` -- a real, if unusual, zero-argument
/// declared function, distinct from `None`'s "infer from source".
fn parse_route_value(raw: &str) -> (&str, Option<Vec<&str>>) {
    let raw = raw.trim();
    if let Some(open) = raw.find('(')
        && raw.ends_with(')')
    {
        let fn_path = raw[..open].trim();
        let inner = raw[open + 1..raw.len() - 1].trim();
        let types = if inner.is_empty() {
            Vec::new()
        } else {
            inner.split(',').map(str::trim).collect()
        };
        return (fn_path, Some(types));
    }
    (raw, None)
}

/// §5.4b's generator hook, resolved: `raw_route` is `routes:`'s own value for
/// `type_name` (`open_handle`, `Token::via_route`, `handles::make_handle`, or
/// -- defect 2 -- `std::ffi::OsString::from(String)`). The local form (no
/// parens) reuses [`discover_fn_with`] -- the exact resolver a `ply.yaml` fn
/// claim's own anchor goes through (§5.5) -- so a route is found (or
/// refused) by the same walk over `use`s, inline `mod`s and file modules
/// every other path in this grammar already is, rather than a second,
/// narrower path-reading scheme with its own bugs to find. `Err` carries the
/// plain-language reason a declared route could not be used -- a function
/// that no longer exists at that path (a stale route, renamed or removed,
/// or one this crate's own source simply never had -- defect 1: a real
/// function outside the crate looks identical to this scan, which has no
/// way to read outside its own crate), one that exists but is not `pub`,
/// one whose return type is not `type_name` at all, or one whose own
/// parameters include something Ply cannot build -- never a bare "not
/// found": the module doc's own "stale route" requirement is that this
/// failure is loud and names what went wrong, not a silent fall-through to
/// rule 2's direct field construction.
fn resolve_declared_route(
    crate_dir: &Path,
    locations: &TypeLocations,
    routes: &RouteTable,
    debug_derivable: bool,
    type_name: &str,
    raw_route: &str,
    depth: usize,
) -> std::result::Result<RustType, String> {
    let (fn_path, declared_types) = parse_route_value(raw_route);

    if let Some(type_strs) = declared_types {
        return resolve_declared_route_typed(
            crate_dir,
            locations,
            routes,
            type_name,
            fn_path,
            &type_strs,
            debug_derivable,
            depth,
        );
    }

    let lib_path = crate_dir.join("src").join("lib.rs");
    let main_path = crate_dir.join("src").join("main.rs");
    let entry_path = if lib_path.is_file() {
        lib_path
    } else {
        main_path
    };
    let mut resolver = resolver_for(&entry_path)
        .map_err(|e| format!("Ply could not read this crate's own source to look for it: {e}"))?;
    let route_cf =
        discover_fn_with(&mut resolver, fn_path, &entry_path).map_err(|e| e.to_string())?;

    let names_it = match (&route_cf.return_type, route_cf.is_method) {
        (RustType::SelfType, true) => enclosing_type_of(&route_cf.path) == Some(type_name),
        (RustType::Unsupported(name), _) => name == type_name,
        _ => false,
    };
    if !names_it {
        return Err(format!(
            "it does not return a value of `{type_name}` -- its return type is `{}`",
            route_cf.return_type.display_name()
        ));
    }

    let mut resolved_params = Vec::with_capacity(route_cf.params.len());
    for p in &route_cf.params {
        match resolve_param_type(crate_dir, locations, routes, &p.ty, depth + 1) {
            Ok(ty) => resolved_params.push(Param { ty, ..p.clone() }),
            Err(e) => {
                return Err(format!(
                    "its `{}: {}` parameter is {}",
                    p.name,
                    p.ty.display_name(),
                    e
                ));
            }
        }
    }

    Ok(RustType::UserTypeCtor(Box::new(ReceiverPlan {
        type_name: type_name.to_string(),
        import_path: route_cf.import_path(),
        constructor: route_cf.call_expr(),
        ctor_params: resolved_params,
        ctor_requires: route_cf.requires.as_ref().map(|(e, _)| e.clone()),
        ctor_return: CtorReturn::Bare,
        operations: vec![],
        excluded_operations: vec![],
        other_constructors: vec![],
        max_sequence_len: 0,
        route: Some(RouteOrigin {
            declared_as: fn_path.to_string(),
            debug_derivable,
            outside_crate: false,
        }),
    })))
}

/// §5.4b's cross-crate extension (defect 2, 2026-09-02): `fn_path` names a
/// function Ply never reads the source of -- `type_strs` are the input
/// types the author wrote instead of the ones Ply would ordinarily infer.
/// Each is built the same way any other parameter of that type would be
/// (recursing through [`resolve_param_type`], so a declared `Vec<String>`
/// or a locally-declared user type both compose exactly as they would
/// anywhere else), then the named path is called directly with them. Ply
/// never reads `fn_path`'s real signature at all here -- not its return
/// type, not its real parameter count or order -- so a mismatch is not
/// caught by this function: it is caught by the compiler when the
/// generated harness tries to build, and reported there as a tool error
/// naming this exact route. That trade -- a wrong signature surfacing as a
/// compiler error rather than a named Ply refusal -- is only honest
/// because the route was explicitly declared; an ordinary unsupported type
/// is never reported this way.
#[allow(clippy::too_many_arguments)]
fn resolve_declared_route_typed(
    crate_dir: &Path,
    locations: &TypeLocations,
    routes: &RouteTable,
    type_name: &str,
    fn_path: &str,
    type_strs: &[&str],
    debug_derivable: bool,
    depth: usize,
) -> std::result::Result<RustType, String> {
    let Some((type_path, _fn_name)) = fn_path.rsplit_once("::") else {
        return Err(format!(
            "`{fn_path}` is not a full path to a function -- write it the way it would be \
             written to call it directly (`some::module::Type::method`, or `some::module::\
             function`)"
        ));
    };
    let mut resolved_params = Vec::with_capacity(type_strs.len());
    for (i, type_str) in type_strs.iter().enumerate() {
        let Some(raw_ty) = rust_type_from_source(type_str) else {
            return Err(format!(
                "its declared input type `{type_str}` does not parse as a Rust type"
            ));
        };
        match resolve_param_type(crate_dir, locations, routes, &raw_ty, depth + 1) {
            Ok(ty) => resolved_params.push(Param {
                name: format!("arg{i}"),
                ty,
                by_ref: false,
            }),
            Err(e) => {
                return Err(format!("its declared input type `{type_str}` is {e}"));
            }
        }
    }
    Ok(RustType::UserTypeCtor(Box::new(ReceiverPlan {
        type_name: type_name.to_string(),
        import_path: type_path.to_string(),
        constructor: fn_path.to_string(),
        ctor_params: resolved_params,
        ctor_requires: None,
        ctor_return: CtorReturn::Bare,
        operations: vec![],
        excluded_operations: vec![],
        other_constructors: vec![],
        max_sequence_len: 0,
        route: Some(RouteOrigin {
            declared_as: fn_path.to_string(),
            debug_derivable,
            outside_crate: true,
        }),
    })))
}

/// Whether the type this crate declares locally as `type_name` has
/// `#[derive(Debug)]` -- computed independently of whichever rule ends up
/// building it (the degenerate-route guard's own honesty condition,
/// [`RouteOrigin::debug_derivable`]), so a declared route can be checked
/// before rule 1's constructor scan even runs (defect 3, 2026-09-02: a
/// route must never be silently shadowed by a rule that happens to find a
/// *different* way to build the same type first). `false` when `type_name`
/// is not declared in this crate at all, or its declaring file cannot be
/// read or parsed -- honest either way: there is no source here to answer
/// from, and a route naming a genuinely external type (§5.4b's cross-crate
/// extension) always falls into exactly this case.
fn locally_declared_type_derives_debug(
    crate_dir: &Path,
    locations: &TypeLocations,
    type_name: &str,
) -> bool {
    let _ = crate_dir;
    let Some(Some(decl)) = locations.get(type_name) else {
        return false;
    };
    let Ok(src) = std::fs::read_to_string(&decl.file) else {
        return false;
    };
    let Ok(file) = syn::parse_file(&src) else {
        return false;
    };
    let mut items: &Vec<syn::Item> = &file.items;
    for m in &decl.inline_mods {
        let Some(next) = items.iter().find_map(|it| match it {
            syn::Item::Mod(md) if md.ident == m.as_str() => md.content.as_ref().map(|(_, i)| i),
            _ => None,
        }) else {
            return false;
        };
        items = next;
    }
    items.iter().any(|it| match it {
        syn::Item::Struct(s) if s.ident == type_name => derives_debug(&s.attrs),
        syn::Item::Enum(e) if e.ident == type_name => derives_debug(&e.attrs),
        _ => false,
    })
}

/// Every declared route this crate is asked to check, exercised regardless
/// of whether any function's parameter actually needs it (defect 1's third
/// required proof, TODO.md: "a route naming a type nothing in the document
/// uses"). `enrich_contract_fn_user_types`'s own per-parameter walk only
/// ever visits a route whose type some parameter actually names -- correct
/// for that walk's own job, but it leaves a route nobody happens to ask for
/// completely unvisited, so a broken one of *this* shape would still pass
/// in total silence. `verify` calls this once per `routes:` entry that its
/// own per-function walk never touched, so every declared route ends up
/// either used or refused by name, never merely unmentioned. `Some` names
/// exactly why (the same sentence a used-and-broken route gets); `None`
/// means the route resolves fine on its own terms.
pub fn validate_declared_route(
    crate_dir: &Path,
    routes: &RouteTable,
    type_name: &str,
) -> Option<String> {
    let locations = scan_crate_type_locations(crate_dir);
    match resolve_user_type(crate_dir, &locations, routes, type_name, 0) {
        Err(UserTypeError::Refused(reason)) => Some(reason),
        _ => None,
    }
}

/// The bare type names among `cf`'s own parameters -- recursively through
/// `Option`/`Result`/`Vec`/`Array`/`Slice`/`Box`/`BTreeSet`/`BTreeMap`/tuple
/// composition and a constructor's or a struct/enum's own nested fields --
/// that a declared route in `routes` actually built, computed *after*
/// [`enrich_contract_fn_user_types`] has run on `cf`. Paired with the
/// `refused` list that same call already returns (a route it tried and
/// could not use), the union of the two is every route this one fn's
/// parameters ever asked about; `verify` accumulates that union across
/// every fn in the crate to know which declared routes still need
/// [`validate_declared_route`]'s own standalone check (TODO.md, "a route
/// naming a type nothing in the document uses").
pub fn route_types_used_by(
    cf: &ContractFn,
    routes: &RouteTable,
) -> std::collections::BTreeSet<String> {
    fn walk(ty: &RustType, routes: &RouteTable, out: &mut std::collections::BTreeSet<String>) {
        match ty {
            RustType::UserTypeCtor(plan) => {
                if routes.contains_key(&plan.type_name) {
                    out.insert(plan.type_name.clone());
                }
                for p in &plan.ctor_params {
                    walk(&p.ty, routes, out);
                }
            }
            RustType::UserTypeFields(plan) => {
                if routes.contains_key(&plan.type_name) {
                    out.insert(plan.type_name.clone());
                }
                match &plan.shape {
                    UserTypeShape::Struct(fields) => {
                        for f in fields {
                            walk(&f.ty, routes, out);
                        }
                    }
                    UserTypeShape::Enum(variants) => {
                        for (_, fields) in variants {
                            for f in fields {
                                walk(&f.ty, routes, out);
                            }
                        }
                    }
                }
            }
            RustType::Option(inner)
            | RustType::Array(inner, _)
            | RustType::Vec(inner)
            | RustType::BTreeSet(inner)
            | RustType::Slice(inner)
            | RustType::BoxT(inner) => walk(inner, routes, out),
            RustType::Result(ok, err) => {
                walk(ok, routes, out);
                walk(err, routes, out);
            }
            RustType::Tuple(items) => {
                for item in items {
                    walk(item, routes, out);
                }
            }
            RustType::BTreeMap(key, value) => {
                walk(key, routes, out);
                walk(value, routes, out);
            }
            _ => {}
        }
    }
    let mut out = std::collections::BTreeSet::new();
    for p in &cf.params {
        walk(&p.ty, routes, &mut out);
    }
    if let Some(plan) = &cf.receiver {
        for p in &plan.ctor_params {
            walk(&p.ty, routes, &mut out);
        }
    }
    out
}

fn resolve_user_type(
    crate_dir: &Path,
    locations: &TypeLocations,
    routes: &RouteTable,
    type_name: &str,
    depth: usize,
) -> std::result::Result<RustType, UserTypeError> {
    if depth > MAX_USER_TYPE_DEPTH {
        return Err(UserTypeError::Refused(format!(
            "`{type_name}` nests more than {MAX_USER_TYPE_DEPTH} user-defined types deep through \
             constructor arguments or fields -- Ply stops following the chain here rather than \
             risk it not terminating"
        )));
    }
    // §5.4b's generator hook, checked first (2026-09-02, defects 1 and 3):
    // a declared route is the author's explicit statement of how to build
    // this type, and it must never be silently shadowed -- neither by a
    // type this scan cannot even find declared in this crate (rule 1/2's
    // door can never open at all: a type built only by a function outside
    // this crate has no local source to scan, defect 1), nor by an
    // unrelated constructor rule 1's own scan happens to find first (a
    // zero-argument, `Self`-returning associated fn is both a legitimate
    // rule-1 constructor *and* the exact shape a route commonly names,
    // defect 3: the route was silently never called, because rule 1 ran
    // first and won without a trace). Trying it here, before rule 1 or rule
    // 2 ever run, means a broken route is always reported as a broken
    // route and a working one always wins -- neither rule ever silently
    // steps in front of a route the author explicitly wrote.
    if let Some(raw_route) = routes.get(type_name) {
        let debug_derivable = locally_declared_type_derives_debug(crate_dir, locations, type_name);
        return resolve_declared_route(
            crate_dir,
            locations,
            routes,
            debug_derivable,
            type_name,
            raw_route,
            depth,
        )
        .map_err(|reason| {
            // A path that plainly lives outside this crate needs the reader
            // pointed at the right fix rather than at their own source. The
            // message underneath names the crate's own library, which is
            // where Ply looked -- true, and exactly the wrong place to send
            // someone whose function is in the standard library. Added
            // 2026-09-02, after the untyped form of a real external route
            // was refused with an accurate sentence that read as "your
            // function is missing".
            let looks_external = !raw_route.contains('(')
                && raw_route.contains("::")
                && !crate::harness::route_path_is_local(locations, raw_route);
            let hint = if looks_external {
                format!(
                    " If `{raw_route}` is a function in another crate rather than this one, \
                     Ply cannot read its source to learn what it takes, so say so in the \
                     route itself -- write `{raw_route}(String)`, naming the type of each \
                     argument, and Ply will build those and call it."
                )
            } else {
                String::new()
            };
            UserTypeError::Refused(format!(
                "Ply cannot build a value of `{type_name}`: the route declared for it in \
                 ply.yaml (`routes: {{ {type_name}: {raw_route} }}`) is broken -- {reason}{hint}"
            ))
        });
    }
    let decl = match locations.get(type_name) {
        Some(Some(d)) => d.clone(),
        Some(None) => return Err(UserTypeError::Ambiguous),
        None => return Err(UserTypeError::NotFound),
    };
    let file_path = decl.file.clone();
    let src = std::fs::read_to_string(&file_path).map_err(|_| UserTypeError::Unreadable)?;
    let file: syn::File = syn::parse_file(&src).map_err(|_| UserTypeError::Unreadable)?;
    let aliases = alias_map(&file);
    // The path the generated harness has to write to name this type, which
    // now includes any inline `mod` it is nested in -- `p8::sub::Only`, not
    // `p8::Only`, which would not compile.
    let import_path = {
        let mut segs = decl.module_segments(crate_dir);
        segs.push(type_name.to_string());
        segs.join("::")
    };
    // Descend the same inline `mod` chain the index recorded, so the
    // declaration found here is the one the index pointed at.
    let mut items: &Vec<syn::Item> = &file.items;
    for m in &decl.inline_mods {
        let Some(next) = items.iter().find_map(|it| match it {
            syn::Item::Mod(md) if md.ident == m.as_str() => md.content.as_ref().map(|(_, i)| i),
            _ => None,
        }) else {
            return Err(UserTypeError::Unreadable);
        };
        items = next;
    }
    let item = items.iter().find(|it| match it {
        syn::Item::Struct(s) => s.ident == type_name,
        syn::Item::Enum(e) => e.ident == type_name,
        _ => false,
    });
    let Some(item) = item else {
        return Err(UserTypeError::Unreadable);
    };

    // Neither rule can name a type the fuzz harness cannot see at all
    // (2026-08-28, docs/review-structs-enums.md's "Also fix" list, "a
    // non-public type" and "a private module"): checked once, before either
    // rule runs, rather than letting a `pub(crate)` type or one sitting
    // behind an unexported module reach codegen and fail to compile there.
    let item_vis = match item {
        syn::Item::Struct(s) => &s.vis,
        syn::Item::Enum(e) => &e.vis,
        _ => unreachable!("the search above only ever matches a Struct or an Enum"),
    };
    if !is_pub(item_vis) {
        return Err(UserTypeError::Refused(format!(
            "Ply cannot build a value of `{type_name}`: the type itself is not `pub`, so the \
             fuzz harness Ply generates -- which sits outside this module -- cannot name it"
        )));
    }
    // A type inside a private inline module is real and unreachable from
    // outside, which is a different answer from "not found" and is said as
    // such rather than folded into the generic refusal.
    if !decl.inline_mods_all_pub {
        return Err(UserTypeError::Refused(format!(
            "Ply cannot build a value of `{type_name}`: it is declared inside an inline `mod` \
             that is not `pub`, so the fuzz harness Ply generates -- which sits outside this \
             crate -- cannot name it by its module path even though the type itself is public. \
             A `pub use` elsewhere may re-export it under a public path; this scan does not \
             follow re-exports yet."
        )));
    }
    if let Some(private_mod) = private_ancestor_module(crate_dir, &file_path) {
        return Err(UserTypeError::Refused(format!(
            "Ply cannot build a value of `{type_name}`: it is declared inside the `{private_mod}` \
             module, which is not `pub`, so the fuzz harness Ply generates cannot name `{type_name}` \
             by its module path even though the type itself is public -- a `pub use` elsewhere in \
             the crate may re-export it under a different, public path, but this scan does not \
             follow re-exports yet"
        )));
    }

    // Rule 1: the type's own constructor, recursively -- the first
    // candidate (source order, declaring file first) whose every parameter
    // itself resolves wins, matching `scan_impls_for_receiver`'s own "first
    // fully-buildable one" preference. Searched across every file in the
    // crate, not only the one `type_name` is declared in (2026-08-28,
    // docs/review-structs-enums.md finding 2, "the constructor lives in a
    // different file from the type") -- and a candidate returning
    // `Result<Self, E>` is a real constructor too (finding 2's other half),
    // never just `Self`.
    // Tracked across the loop so a later rule-2 success can say honestly
    // that a constructor exists but was not used, rather than silently
    // building field-by-field as though none did (2026-08-28,
    // docs/review-structs-enums.md finding 2, "is the disclosure enough? --
    // No": the old wording ("it has no constructor Ply can call") is true
    // only when this stays `None`).
    let mut skipped_constructor: Option<String> = None;
    // `impl Default for T` is a constructor anyone can call, and the scans
    // above skip every trait `impl`, so a type whose only way to be built
    // is `T::default()` was told it had none -- false, and the whole family
    // of bug this file has been correcting today. Building through it is
    // separate, larger work (a single value is not 256 cases, however many
    // times it is run: TODO.md carries the design), so what changes here is
    // the sentence, which was wrong regardless of whether the construction
    // ever lands.
    if type_declares_default(&file, type_name, &decl.inline_mods) {
        skipped_constructor = Some(format!(
            "`{type_name}::default()`, which Ply does not build values through yet: it \
             produces a single value, and reporting that as many sampled cases would \
             overstate what was checked"
        ));
    }
    for (ctor_path, raw_params, ctor_requires, ctor_return, ctor_is_pub) in
        scan_ctor_candidates_crate_wide(crate_dir, &file_path, type_name)
    {
        // A private constructor is never callable from the fuzz harness Ply
        // generates outside this crate (2026-08-28, docs/review-structs-
        // enums.md's "Also fix" list, "a private constructor"): checked
        // before spending any recursion on its parameters, and reported
        // through the same `skipped_constructor` channel as an unbuildable
        // one -- found, named, and explained, never silently treated as
        // though no constructor existed.
        let mut fail_reason = if ctor_is_pub {
            None
        } else {
            Some(
                "it is private, and the harness Ply generates lives outside this crate, so it \
                  cannot call it"
                    .to_string(),
            )
        };
        let mut resolved_params = Vec::with_capacity(raw_params.len());
        if fail_reason.is_none() {
            for p in &raw_params {
                match resolve_param_type(crate_dir, locations, routes, &p.ty, depth + 1) {
                    Ok(ty) => resolved_params.push(Param { ty, ..p.clone() }),
                    Err(e) => {
                        fail_reason = Some(format!(
                            "its `{}: {}` parameter is {}",
                            p.name,
                            p.ty.display_name(),
                            e
                        ));
                        break;
                    }
                }
            }
        }
        match fail_reason {
            None => {
                return Ok(RustType::UserTypeCtor(Box::new(ReceiverPlan {
                    type_name: type_name.to_string(),
                    import_path: import_path.clone(),
                    constructor: ctor_path,
                    ctor_params: resolved_params,
                    ctor_requires,
                    ctor_return,
                    operations: vec![],
                    excluded_operations: vec![],
                    other_constructors: vec![],
                    max_sequence_len: 0,
                    route: None,
                })));
            }
            Some(reason) if skipped_constructor.is_none() => {
                skipped_constructor = Some(format!(
                    "`{ctor_path}`, which Ply could not use because {reason}"
                ));
            }
            Some(_) => {}
        }
    }

    // §5.4b's generator hook is no longer tried here: a declared route is
    // now checked at the very top of this function, before rule 1 even
    // runs (2026-09-02, defects 1 and 3) -- so by this point, either no
    // route was declared for `type_name` at all, or it was and this
    // function already returned through it. `item` (below) is used only by
    // rule 1/2 from here on.

    // Every refusal below used to open "it has no constructor Ply can
    // call". That is true only while `skipped_constructor` is `None`. A
    // constructor Ply found and could not use -- private, or with an
    // argument it cannot build, `fn new(n: impl Into<u32>)` being the
    // ordinary case -- was recorded here and then dropped by every refusal
    // arm, so a user with a perfectly public `new` two lines away was told
    // it did not exist (2026-08-28, review finding). The note now replaces
    // that clause wherever it exists, so the sentence names what was found
    // and why it could not be used.
    //
    // No route was declared for this type either (the `if let Some` above
    // already returned when one was) -- so the refusal names the one thing
    // that would still unlock it, rather than leaving a reader to
    // rediscover §5.4b's generator hook on their own.
    let no_ctor = |extra: &str| -> UserTypeError {
        let opening = match &skipped_constructor {
            None => format!("it has no constructor Ply can call, {extra}"),
            Some(note) => format!("the only constructor it has is {note}; and {extra}"),
        };
        UserTypeError::Refused(format!(
            "Ply cannot build a value of `{type_name}`: {opening} -- declaring a route for it in \
             ply.yaml (`routes: {{ {type_name}: <a public function that returns {type_name}> }}`) \
             would let Ply build it"
        ))
    };

    // Rule 2: direct construction, only when nothing is private and
    // nothing is a shape this reader does not recognise.
    match item {
        syn::Item::Struct(s) => {
            if has_non_exhaustive(&s.attrs) {
                return Err(no_ctor(
                    "it is marked `#[non_exhaustive]`, which blocks a field literal from \
                     outside its own crate -- the fuzz harness Ply generates is exactly such \
                     an outside crate",
                ));
            }
            if !all_fields_public(&s.fields) {
                let private: Vec<String> = s
                    .fields
                    .iter()
                    .filter(|f| !matches!(f.vis, syn::Visibility::Public(_)))
                    .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
                    .collect();
                let which = if private.is_empty() {
                    "one or more of its fields are".to_string()
                } else {
                    format!("field(s) {} are", private.join(", "))
                };
                return Err(no_ctor(&format!(
                    "{which} private, so building it field by field would risk a value the \
                     real program could never produce"
                )));
            }
            let Some(fields) = named_fields_as_params(&s.fields, &aliases) else {
                return Err(no_ctor(
                    "its fields are positional (a tuple struct), a shape direct construction \
                     does not read yet",
                ));
            };
            if fields.len() > MAX_DIRECT_CONSTRUCTION_FIELDS {
                return Err(no_ctor(&format!(
                    "it has {} public fields -- direct field construction's generated strategy \
                     is a tuple of one value per field, and the trait proptest builds that on \
                     stops being implemented past {MAX_DIRECT_CONSTRUCTION_FIELDS} \
                     (2026-08-28, docs/review-structs-enums.md's \"Also fix\" list)",
                    fields.len()
                )));
            }
            let mut resolved = Vec::with_capacity(fields.len());
            for f in fields {
                match resolve_param_type(crate_dir, locations, routes, &f.ty, depth + 1) {
                    Ok(ty) => resolved.push(Param { ty, ..f }),
                    Err(e) => {
                        return Err(no_ctor(&format!(
                            "field `{}` has type `{}`, which is {}",
                            f.name,
                            f.ty.display_name(),
                            e
                        )));
                    }
                }
            }
            Ok(RustType::UserTypeFields(Box::new(UserTypeFieldsPlan {
                type_name: type_name.to_string(),
                import_path: import_path.clone(),
                shape: UserTypeShape::Struct(resolved),
                skipped_constructor: skipped_constructor.clone(),
            })))
        }
        syn::Item::Enum(e) => {
            if has_non_exhaustive(&e.attrs) {
                return Err(no_ctor(
                    "it is marked `#[non_exhaustive]`, which blocks building a variant from \
                     outside its own crate -- the fuzz harness Ply generates is exactly such \
                     an outside crate",
                ));
            }
            if e.variants.is_empty() {
                return Err(UserTypeError::Refused(format!(
                    "`{type_name}` declares no variants at all -- there is no value to build"
                )));
            }
            let mut variants = Vec::with_capacity(e.variants.len());
            for v in &e.variants {
                // `#[non_exhaustive]` on a *variant* rather than the enum
                // itself (2026-08-28, docs/review-structs-enums.md's "Also
                // fix" list): `has_non_exhaustive(&e.attrs)` above only ever
                // reads the enum's own attributes, so this ordinary Rust
                // shape (a stable enum whose newer variants are individually
                // marked unstable) reached codegen unrefused and failed to
                // compile there (`error[E0639]: cannot create non-exhaustive
                // variant using struct expression`). Refusing the whole
                // enum, not just this variant, matches the enum-level
                // discipline just above and `UserTypeShape`'s own doc: a
                // harness that silently drops one variant under-represents
                // the type without saying so.
                if has_non_exhaustive(&v.attrs) {
                    return Err(no_ctor(&format!(
                        "variant `{}` is marked `#[non_exhaustive]`, which blocks building that \
                         variant from outside its own crate -- refusing the whole enum rather \
                         than silently building only some of its variants",
                        v.ident
                    )));
                }
                let Some(fields) = named_fields_as_params(&v.fields, &aliases) else {
                    return Err(UserTypeError::Refused(format!(
                        "Ply cannot build a value of `{type_name}`: variant `{}` has positional \
                         (tuple) fields, a shape direct construction does not read yet -- \
                         refusing the whole enum rather than silently building only some of its \
                         variants",
                        v.ident
                    )));
                };
                let mut resolved = Vec::with_capacity(fields.len());
                for f in fields {
                    match resolve_param_type(crate_dir, locations, routes, &f.ty, depth + 1) {
                        Ok(ty) => resolved.push(Param { ty, ..f }),
                        Err(e2) => {
                            return Err(UserTypeError::Refused(format!(
                                "Ply cannot build a value of `{type_name}`: variant `{}`'s \
                                 field `{}` has type `{}`, which is {}",
                                v.ident,
                                f.name,
                                f.ty.display_name(),
                                e2
                            )));
                        }
                    }
                }
                variants.push((v.ident.to_string(), resolved));
            }
            Ok(RustType::UserTypeFields(Box::new(UserTypeFieldsPlan {
                type_name: type_name.to_string(),
                import_path: import_path.clone(),
                shape: UserTypeShape::Enum(variants),
                skipped_constructor,
            })))
        }
        _ => unreachable!("the search above only ever matches a Struct or an Enum"),
    }
}

/// The parameter counterpart of receiver construction: upgrades every
/// parameter of `cf` whose type parsed to `RustType::Unsupported` and names
/// a bare struct/enum this crate declares into a value Ply itself knows how
/// to build, in place. Returns `(param_name, type_name, reason)` for every
/// parameter this scan recognised as a real struct/enum declaration but
/// still could not build (rule 3's refusal) -- a parameter whose
/// `Unsupported` type is not a struct/enum this crate declares at all is
/// left exactly as it was, silently: the generic "type neither engine
/// builds inputs for" diagnostic already names it honestly, and reporting
/// "not found" about something that was never a candidate in the first
/// place would not be true.
/// The bare name to look a written parameter type up by, but **only** when
/// the path as written and the declaration Ply would find are the same
/// type.
///
/// A bare `Beta` is itself. A qualified `crate::Beta` or `bucket::Quota` is
/// accepted when the qualifiers match the module the type is actually
/// declared in. Anything else -- `depx::Thing` naming a dependency's type
/// while this crate declares its own `Thing`, or a `super::` chain with no
/// module context to resolve it against -- returns `None`, so the parameter
/// stays refused with the path the user wrote, rather than being built as
/// some other type that merely shares its last segment.
///
/// This exists because the first version of the qualified-path fix trimmed
/// every path to its last segment. That made `v: depx::Thing` resolve to
/// the local `Thing`, generate a harness that could not compile, and report
/// a tool error blaming Ply's own generated code -- turning a calm, correct
/// refusal into an internal error. Reported by review, reproduced, fixed
/// here (2026-08-28).
fn crate_local_type_name(
    crate_dir: &Path,
    locations: &TypeLocations,
    written: &str,
) -> Option<String> {
    if is_bare_ident(written) {
        return Some(written.to_string());
    }
    let mut segs: Vec<&str> = written.split("::").collect();
    let name = segs.pop()?;
    if !is_bare_ident(name) {
        return None;
    }
    // `crate::`/`self::` at the front just says "from the crate root",
    // which is where the module walk below starts anyway. `super::` is
    // relative to the writing module, which this scan does not have here,
    // so it is refused rather than guessed at.
    if matches!(segs.first(), Some(&"crate") | Some(&"self")) {
        segs.remove(0);
    }
    if segs
        .iter()
        .any(|s| *s == "super" || *s == "crate" || *s == "self")
    {
        return None;
    }
    // Where the type of that name actually lives. Declared nowhere, or in
    // more than one file, and no qualified spelling can be confirmed.
    let declared_in = match locations.get(name) {
        Some(Some(path)) => path.clone(),
        _ => return None,
    };
    let declared_mod = declared_in.module_segments(crate_dir);
    (declared_mod == segs).then(|| name.to_string())
}

pub fn enrich_contract_fn_user_types(
    cf: &mut ContractFn,
    crate_dir: &Path,
    routes: &RouteTable,
) -> Vec<(String, String, String)> {
    let locations = scan_crate_type_locations(crate_dir);
    // A parameter written `&Self` parses (`rust_type_from_syn_at`) to
    // `Unsupported("Self")` -- "Self" is just another bare identifier to
    // that parser, which has no notion of which `impl` block it sits in.
    // But it names the *same* type the receiver is a value of, and the
    // receiver's own plan has already resolved that type once
    // (`ReceiverPlan::type_name`, set by `scan_impls_for_receiver`) -- so a
    // `Self` parameter is substituted with that name before the ordinary
    // by-name lookup below runs, rather than being left to fail it (no
    // struct/enum is ever actually named "Self"). This is the same
    // resolution the receiver already goes through, reused rather than
    // re-derived: a method with no receiver plan attached (a free function,
    // never inside an `impl` block) has no `Self` to resolve against, so
    // the parameter is left as found.
    let self_type_name = cf.receiver.as_ref().map(|r| r.type_name.clone());
    let mut refused = Vec::new();
    for p in &mut cf.params {
        enrich_rust_type(
            &mut p.ty,
            crate_dir,
            &locations,
            routes,
            &p.name,
            self_type_name.as_deref(),
            &mut refused,
        );
    }
    refused
}

/// [`enrich_contract_fn_user_types`]'s own recursion, added 2026-09-02 for
/// composition (TODO.md): a bare top-level parameter is all this ever
/// walked before, so `Vec<Doc>` -- parsed, before this task, straight to one
/// opaque `Unsupported("Vec<Doc>")` string, never a composite with a `Doc`
/// inside it to resolve -- never had anything here to find. Now that the
/// parser preserves structure through every composition shape (`Option`,
/// `Result`, `[T; N]`, `Vec`, `BTreeSet`, `BTreeMap`, `&[T]`, a tuple,
/// `Box` -- see `is_composable`'s own doc), this walks into all of them,
/// resolving every nested `Unsupported(bare-ident)` leaf it finds exactly
/// the way the old top-level-only version resolved the one at the root.
/// Any leaf that does not resolve (an unrelated type, or a real struct/enum
/// rule 3 refuses) is simply left `Unsupported` where it sits -- the
/// ordinary support predicates already propagate `false` up through it, and
/// a genuine rule-3 refusal is still reported, named by the parameter it
/// came from.
/// `self_type_name` is the enclosing `impl`'s own type, so a parameter written
/// as the shorthand `Self` resolves to exactly what naming that type would
/// resolve to (fixed 2026-09-01; the same function was `unsupported` spelled
/// one way and checked spelled the other). It is threaded through the
/// recursion rather than handled only at the top, so `Option<Self>` and
/// `Vec<Self>` resolve too -- the shapes this recursion exists to reach.
/// Whether a route's path names something this crate itself declares. Used
/// only to decide how to word a refusal: a path we cannot find AND cannot
/// see locally is almost certainly in another crate, where the fix is to
/// declare the argument types rather than to go looking in your own source.
pub(crate) fn route_path_is_local(locations: &TypeLocations, raw_route: &str) -> bool {
    let head = raw_route.split("::").next().unwrap_or(raw_route);
    locations.contains_key(head)
}

fn enrich_rust_type(
    ty: &mut RustType,
    crate_dir: &Path,
    locations: &TypeLocations,
    routes: &RouteTable,
    param_name: &str,
    self_type_name: Option<&str>,
    refused: &mut Vec<(String, String, String)>,
) {
    match ty {
        RustType::Unsupported(src) => {
            let src = if src == "Self" {
                let Some(name) = self_type_name else {
                    return;
                };
                name.to_string()
            } else {
                src.clone()
            };
            // A declared route names a type by its bare name alone (model.rs:
            // "keyed by the bare type name... not a module-qualified path"),
            // so it is checked directly off the last path segment here --
            // *before* `crate_local_type_name`'s own module-matching check,
            // which would otherwise refuse a genuinely external path
            // (`std::ffi::OsString`, written out in full because there is no
            // local declaration to match it against) before a route for it
            // is ever tried (defect 1, 2026-09-02).
            let bare_name = src.rsplit("::").next().unwrap_or(&src);
            let name = if routes.contains_key(bare_name) {
                bare_name.to_string()
            } else {
                let Some(name) = crate_local_type_name(crate_dir, locations, &src) else {
                    return;
                };
                name
            };
            match resolve_user_type(crate_dir, locations, routes, &name, 0) {
                Ok(resolved) => *ty = resolved,
                Err(UserTypeError::NotFound) => {}
                Err(e) => refused.push((param_name.to_string(), name, e.to_string())),
            }
        }
        RustType::Option(inner)
        | RustType::Array(inner, _)
        | RustType::Vec(inner)
        | RustType::BTreeSet(inner)
        | RustType::Slice(inner)
        | RustType::BoxT(inner) => {
            enrich_rust_type(
                inner,
                crate_dir,
                locations,
                routes,
                param_name,
                self_type_name,
                refused,
            );
        }
        RustType::Result(ok, err) => {
            enrich_rust_type(
                ok,
                crate_dir,
                locations,
                routes,
                param_name,
                self_type_name,
                refused,
            );
            enrich_rust_type(
                err,
                crate_dir,
                locations,
                routes,
                param_name,
                self_type_name,
                refused,
            );
        }
        RustType::Tuple(items) => {
            for item in items {
                enrich_rust_type(
                    item,
                    crate_dir,
                    locations,
                    routes,
                    param_name,
                    self_type_name,
                    refused,
                );
            }
        }
        RustType::BTreeMap(key, value) => {
            enrich_rust_type(
                key,
                crate_dir,
                locations,
                routes,
                param_name,
                self_type_name,
                refused,
            );
            enrich_rust_type(
                value,
                crate_dir,
                locations,
                routes,
                param_name,
                self_type_name,
                refused,
            );
        }
        _ => {}
    }
}

/// One callee stubbed out of a proof under D5's second branch (§5.5): its
/// contract is *declared* (in `ply.yaml`) but nothing has verified it, so
/// Ply replaces the callee with a function that returns an arbitrary value
/// constrained by the declared `ensures`, and asserts the declared
/// `requires` at the call. That is the whole content of "assume the
/// contract": the caller is proved against the promise, never against the
/// body -- which is what makes the resulting verdict `conditional` rather
/// than `bounded` full stop.
/// Which mechanism a stub renders -- and, for a same-crate contracted
/// callee, which of D5's two branches (§5.5) Ply's own ordering decided.
///
/// Two *mechanisms*, not two branches: `Assumed` is for a callee that
/// carries **no** inline contract of its own (§5.5's second branch reached
/// through a `ply.yaml`-declared contract, D2's boundary-contract route) --
/// Kani's plain `#[kani::stub]` works directly there. `Contracted` is for a
/// same-crate callee that **does** carry its own inline `#[ply::requires]`/
/// `#[ply::ensures]`: Kani's plain `#[kani::stub]` cannot target a
/// contracted function at all (Kani issue #4591, reproduced against both
/// the pinned toolchain and Kani `main`, `tests/spike/kani-pin`'s blocker
/// 2, and again directly against this feature 2026-08-26 -- "Failed to
/// find contract closure" is a **compile** error, killing the whole
/// crate), so `#[kani::stub_verified]` plus a never-run "existence" harness
/// is the *only* mechanism Kani offers for such a target, and both of D5's
/// branches use it identically. What tells them apart is `bound`: `Some(k)`
/// only when Ply's own ordering established this run that the callee
/// earned a clean `bounded(k)` (branch one -- not `conditional`, owes
/// nothing, composes the caller's bound to `min`); `None` when it could
/// not (a cycle, or the callee's own check did not come back clean this
/// run) -- branch two, exactly as `conditional` always meant, mechanically
/// indistinguishable to Kani (`stub_verified`'s own check is purely
/// syntactic either way, tests/spike's finding 1 -- Ply's scheduler is the
/// entire soundness argument, never Kani's).
#[derive(Debug, Clone)]
pub enum StubKind {
    /// No inline contract on the callee -- a hand-built stand-in function
    /// plus plain `#[kani::stub]`. The caller's verdict is `conditional`
    /// (`W0511`) and the assumption is owed evidence.
    Assumed,
    /// A same-crate callee carrying its own inline contract. `params` are
    /// its own normalised parameters, needed only to render the never-run
    /// "existence" harness `render_existence` emits alongside --
    /// `#[kani::stub_verified]` requires *some*
    /// `#[kani::proof_for_contract(g)]` harness to be present in the
    /// compiled crate, and checks nothing about whether it ran or passed.
    Contracted {
        bound: Option<u32>,
        params: Vec<Param>,
    },
}

#[derive(Debug, Clone)]
pub struct StubSpec {
    /// The callee path exactly as the caller writes it -- also
    /// `#[kani::stub(..)]`'s first argument.
    pub callee_path: String,
    /// `(name, type source)` in declaration order, taken from the callee's
    /// real signature so a rendered stand-in fn is signature-compatible
    /// (Kani checks) -- populated for both branches, since `crate::promise`
    /// ranges a `requires` probe over these regardless of `kind`. Not what
    /// `render_existence()` uses for its own `kani::any()` bindings, though:
    /// those need the *dereferenced* type (`Contracted::params`, already
    /// normalised), while this field keeps the raw, possibly-referenced
    /// text a stand-in function's own signature needs.
    pub params: Vec<(String, String)>,
    pub return_type: String,
    pub requires: Vec<String>,
    pub ensures: Vec<String>,
    pub kind: StubKind,
}

impl StubSpec {
    /// True for D5's second branch (§5.5): the caller is `conditional` and
    /// owes evidence for this callee. False only for `Contracted { bound:
    /// Some(_), .. }` (branch one) -- real evidence the caller does not owe
    /// anything for.
    pub fn is_assumed(&self) -> bool {
        !matches!(self.kind, StubKind::Contracted { bound: Some(_), .. })
    }

    /// The `bounded(k)` this callee's own proof earned this run, when this
    /// stub is D5's first branch (§5.5) -- `None` for `Assumed` and for a
    /// `Contracted` stub that fell back to branch two.
    pub fn verified_bound(&self) -> Option<u32> {
        match &self.kind {
            StubKind::Contracted { bound, .. } => *bound,
            StubKind::Assumed => None,
        }
    }
    /// A deterministic Rust identifier for the generated stub fn.
    pub fn stub_fn_name(&self) -> String {
        format!("ply_stub_{}", self.callee_path.replace("::", "_"))
    }

    /// The one-line description of the assumption this stub encodes, for
    /// `W0511` and the §8 `assumptions` list.
    pub fn assumption_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for r in &self.requires {
            parts.push(format!("requires {r}"));
        }
        for e in &self.ensures {
            parts.push(format!("ensures {e}"));
        }
        if parts.is_empty() {
            format!("`{}` (contract declared with no clauses)", self.callee_path)
        } else {
            format!("`{}`: {}", self.callee_path, parts.join(", "))
        }
    }

    fn render(&self) -> Result<String> {
        let params: Vec<String> = self
            .params
            .iter()
            .map(|(n, ty)| format!("{n}: {ty}"))
            .collect();
        let mut body = String::new();
        for r in &self.requires {
            let expr: Expr = syn::parse_str(r).with_context(|| {
                format!(
                    "E0501: could not parse the `requires` declared for `{}` as an expression: {r}",
                    self.callee_path
                )
            })?;
            let text = expr.to_token_stream().to_string();
            body.push_str(&format!(
                "    kani::assert({text}, \"the caller must satisfy the contract declared for `{path}`\");\n",
                path = self.callee_path
            ));
        }
        body.push_str(&format!(
            "    let __ply_result: {ret} = kani::any();\n",
            ret = self.return_type
        ));
        for e in &self.ensures {
            let closure: ExprClosure = syn::parse_str(e).with_context(|| {
                format!(
                    "E0501: the `ensures` declared for `{}` must be a `|result| expr` closure, got: {e}",
                    self.callee_path
                )
            })?;
            // The closure parameter needs an explicit type: applied to a
            // reference with nothing else to infer from, rustc reports
            // "type annotations needed" and the harness never compiles.
            let mut inputs = closure.inputs.iter();
            let pat = match inputs.next() {
                Some(p) => p.to_token_stream().to_string(),
                None => bail!(
                    "E0501: the `ensures` declared for `{}` takes no parameter -- it must be a \
                     `|result| expr` closure",
                    self.callee_path
                ),
            };
            let cbody = closure.body.to_token_stream().to_string();
            body.push_str(&format!(
                "    kani::assume((|{pat}: &{ret}| {cbody})(&__ply_result));\n",
                ret = self.return_type
            ));
        }
        body.push_str("    __ply_result\n");
        Ok(format!(
            "#[cfg(kani)]\n\
             #[allow(dead_code, unused_variables)]\n\
             fn {name}({params}) -> {ret} {{\n\
             {body}}}\n",
            name = self.stub_fn_name(),
            params = params.join(", "),
            ret = self.return_type,
            body = body,
        ))
    }

    /// A `StubKind::Contracted` stub (§5.5): a harness that calls the real
    /// callee with symbolic arguments and carries
    /// `#[kani::proof_for_contract(..)]` for it, so that Kani's
    /// compile-time existence check for `#[kani::stub_verified]` is
    /// satisfied. Never named in `--harness`, so it never actually runs
    /// here -- for a branch-one stub, the callee's own separate run
    /// earlier this pass (or a still-valid record, D5's honesty condition
    /// 3 above) is what actually proved it; for a branch-two stub (a
    /// cycle, or the callee's own check did not come back clean this run)
    /// nothing did, and that is exactly why the caller stays `conditional`
    /// -- Kani's own check here cannot tell the two apart, only Ply's
    /// bookkeeping can. `params` is the callee's own normalised signature
    /// (not `self.params`, which is `Assumed`'s raw, possibly-referenced
    /// text -- see the field's own doc comment).
    fn render_existence(&self, params: &[Param]) -> String {
        let (lets, call_args) = render_kani_args(params, 1);
        let name = format!(
            "ply_verified_exists_{}",
            self.callee_path.replace("::", "_")
        );
        format!(
            "#[cfg(kani)]\n\
             #[allow(dead_code, unused_variables)]\n\
             #[kani::proof_for_contract({path})]\n\
             fn {name}() {{\n\
             {lets}\
             \x20\x20\x20\x20{path}({args});\n\
             }}\n",
            path = self.callee_path,
            args = call_args.join(", "),
        )
    }
}

/// The generated Kani proof module for one `ContractFn`.
pub struct GeneratedHarness {
    /// The full generated-file source (`ply_generated.rs`'s content).
    pub module_source: String,
    /// The `--harness` path Kani needs (`ply_generated::ply_proof_<fn>`).
    pub proof_fn_path: String,
    /// The bound Kani's `#[kani::unwind(..)]` was emitted with, if any
    /// `Vec`-typed parameter is present. `None` means no Vec parameter and
    /// therefore no unwind annotation was needed.
    pub unwind: Option<u32>,
    /// Every callee this harness stubbed, either branch (§5.5), in the
    /// order they appear in the proof's attributes. Non-empty means the run
    /// needs Kani's `-Z stubbing`; the verdict is `conditional` only if any
    /// entry is `StubKind::Assumed` (`is_assumed()`) -- a callee stubbed
    /// `Verified` is real evidence the caller does not owe anything for.
    pub stubbed: Vec<StubSpec>,
    /// The promise-content probes generated beside the proof: one harness
    /// per question Ply asks about each declared clause (§5.5, `crate::promise`).
    /// They ride in the same generated module so the crate compiles once for
    /// all of them.
    pub promise: crate::promise::PromisePlan,
}

/// Generates the `#[kani::proof_for_contract]` harness for `cf`, sized by
/// `bound_k` (the declared `bounded(k)` -- also used as the Vec length bound
/// when the function has a `Vec<u8>` parameter). Emits `#[kani::unwind(k+1)]`
/// whenever a Vec parameter is present -- §5.4b's mandatory annotation,
/// measured (not inferred) for exactly this manual-indexed-loop-consumption
/// shape in docs/m3-slice-findings.md. Without it, Kani's default unwind
/// inference times out at every length, including 1.
/// Builds `kani::any()` (or `kani::vec::any_vec`) bindings for `params` at
/// `bound_k`, plus the call-site arguments (`&x` for a by-ref param) --
/// the one place this shape is built, shared between a claimed fn's own
/// proof and D5's first branch (§5.5): the never-run "existence" harness
/// that stands in for a `#[kani::stub_verified]` target's own
/// `#[kani::proof_for_contract]` requirement (tests/spike's finding 1 --
/// Kani's check is purely that such a harness is present in the same
/// compiled crate, never that it ran or passed here).
fn render_kani_args(params: &[Param], bound_k: u32) -> (String, Vec<String>) {
    let mut lets = String::new();
    let mut call_args = Vec::new();
    for p in params {
        match &p.ty {
            RustType::VecU8 => {
                lets.push_str(&format!(
                    "    let {name} = kani::vec::any_vec::<u8, {n}>();\n",
                    name = p.name,
                    n = bound_k
                ));
            }
            // Never `kani::any::<NonZeroU32>()` directly: this codegen does
            // not rely on Kani's own `Arbitrary` impl for the type to
            // itself forbid zero. The inner integer is what is actually
            // symbolic; `kani::assume` rules zero out before the `NonZero`
            // is ever constructed, so the constraint reaches the solver
            // rather than living only in the type's own (untrusted, here)
            // promise. A generated value that could be zero would let the
            // proof explore a state the type forbids -- a witness that
            // could never occur for a real caller (task brief, 2026-08-27).
            RustType::NonZero(inner) => {
                let inner_ty = inner
                    .rust_name()
                    .expect("nonzero inner is always a plain integer");
                lets.push_str(&format!(
                    "    let {name}_inner: {inner_ty} = kani::any();\n\
                     \x20\x20\x20\x20kani::assume({name}_inner != 0);\n\
                     \x20\x20\x20\x20let {name}: std::num::NonZero{suffix} = \
                     std::num::NonZero{suffix}::new({name}_inner).unwrap();\n",
                    name = p.name,
                    inner_ty = inner_ty,
                    suffix = inner.nonzero_suffix().expect("checked supported above"),
                ));
            }
            // Two independent scalars, not a derived struct: `Duration`'s
            // fields are private, so §5.4b's struct path (Ply-derivable
            // Arbitrary over public fields) cannot see them at all. The
            // `assume` on `nanos` is the type's own real invariant -- the
            // standard library never returns a `Duration` whose internal
            // nanos field reaches a billion -- not a loop bound, so unlike
            // `Vec` there is no unwind annotation to emit here, and no
            // second bound to surface in the verdict: `secs` ranges over
            // the whole `u64` with no assume at all, exactly as it would if
            // Ply were asked to build a bare `u64` parameter.
            RustType::Duration => {
                lets.push_str(&format!(
                    "    let {name}_secs: u64 = kani::any();\n\
                     \x20\x20\x20\x20let {name}_nanos: u32 = kani::any();\n\
                     \x20\x20\x20\x20kani::assume({name}_nanos < 1_000_000_000u32);\n\
                     \x20\x20\x20\x20let {name}: std::time::Duration = \
                     std::time::Duration::new({name}_secs, {name}_nanos);\n",
                    name = p.name,
                ));
            }
            other => {
                let ty_name = other.rust_name().expect("checked supported above");
                lets.push_str(&format!(
                    "    let {name}: {ty} = kani::any();\n",
                    name = p.name,
                    ty = ty_name
                ));
            }
        }
        call_args.push(if p.by_ref {
            format!("&{}", p.name)
        } else {
            p.name.clone()
        });
    }
    (lets, call_args)
}

pub fn generate_proof_module(
    cf: &ContractFn,
    bound_k: u32,
    stubs: &[StubSpec],
) -> Result<GeneratedHarness> {
    if !cf.is_bounded_supported() {
        let bad: Vec<String> = cf
            .params
            .iter()
            .filter(|p| !p.ty.is_bounded_supported())
            .map(|p| format!("{}: {:?}", p.name, p.ty))
            .collect();
        bail!(
            "V0505: unsupported parameter type(s) for `{}`: {}",
            cf.name,
            bad.join(", ")
        );
    }

    let has_vec = cf.has_vec_param();
    let (lets, call_args) = render_kani_args(&cf.params, bound_k);

    let unwind = if has_vec { Some(bound_k + 1) } else { None };
    let unwind_attr = unwind
        .map(|n| format!("#[kani::unwind({n})]\n"))
        .unwrap_or_default();

    // D5's second branch's own callees only -- a `StubKind::Contracted`
    // proved this run stands on real evidence, so its inline contract is
    // not an assumption to interrogate for vacuity at all (§5.5: "does not
    // look at a verified function's own inline `#[ply::ensures]`"). Before
    // this filter existed the gate ran on branch one's callees too, so a
    // trivially-true `ensures` on a *proved* callee failed the whole run
    // with an `E0503` naming a promise that was never assumed (adversarial
    // review, 2026-08-26).
    let assumed_only: Vec<StubSpec> = stubs.iter().filter(|s| s.is_assumed()).cloned().collect();
    let promise = crate::promise::plan(&assumed_only);
    let mut stub_defs = String::new();
    let mut stub_attrs = String::new();
    for s in stubs {
        match &s.kind {
            StubKind::Assumed => {
                stub_defs.push_str(&s.render()?);
                stub_defs.push('\n');
                stub_attrs.push_str(&format!(
                    "#[kani::stub({path}, {name})]\n",
                    path = s.callee_path,
                    name = s.stub_fn_name()
                ));
            }
            StubKind::Contracted { params, .. } => {
                stub_defs.push_str(&s.render_existence(params));
                stub_defs.push('\n');
                stub_attrs.push_str(&format!(
                    "#[kani::stub_verified({path})]\n",
                    path = s.callee_path
                ));
            }
        }
    }

    let proof_fn_name = format!("ply_proof_{}", cf.ident());
    let module_source = format!(
        "//! Generated by Ply -- do not edit. Kani proof harness for `{fname}`\n\
         //! (check bounded({k})). See The-Ply-Spec.md D2 and §5.4b.\n\
         #[cfg(kani)]\n\
         use super::*;\n\n\
         {stub_defs}\
         #[cfg(kani)]\n\
         #[kani::proof_for_contract({fname})]\n\
         {stub_attrs}\
         {unwind_attr}\
         fn {proof_fn_name}() {{\n\
         {lets}\
         \x20\x20\x20\x20{fname}({args});\n\
         }}\n\
         {promise_defs}",
        fname = cf.path,
        k = bound_k,
        stub_defs = stub_defs,
        stub_attrs = stub_attrs,
        unwind_attr = unwind_attr,
        proof_fn_name = proof_fn_name,
        lets = lets,
        args = call_args.join(", "),
        promise_defs = promise.source(),
    );

    Ok(GeneratedHarness {
        module_source,
        proof_fn_path: format!("ply_generated::{proof_fn_name}"),
        unwind,
        stubbed: stubs.to_vec(),
        promise,
    })
}

/// Writes the generated harness file into `crate_src_dir` (a crate's `src/`
/// directory) as `ply_generated.rs`, and idempotently ensures the crate's
/// `lib_path` declares `mod ply_generated;` -- the exact "generated file plus
/// one module declaration" mechanism D2 describes. In-crate placement is
/// load-bearing: it lets the harness (and later, the rendered cex test) see
/// private items (ADR-0003 item 1).
pub fn write_generated_module(
    crate_src_dir: &Path,
    lib_path: &Path,
    module_source: &str,
) -> Result<PathBuf> {
    write_generated_file(crate_src_dir, lib_path, "ply_generated", module_source)
}

/// Writes the D7 rendered cex test(s) into `crate_src_dir` as
/// `ply_generated_cex.rs`, declared from `lib_path` the same way as the
/// proof module -- same in-crate mechanism, same reason (private-item
/// visibility, ADR-0003 item 1). Each item inside is already `#[cfg(test)]`
/// so the outer `mod` declaration itself needs no gating.
pub fn write_generated_test(
    crate_src_dir: &Path,
    lib_path: &Path,
    test_module_source: &str,
) -> Result<PathBuf> {
    write_generated_file(
        crate_src_dir,
        lib_path,
        "ply_generated_cex",
        test_module_source,
    )
}

fn write_generated_file(
    crate_src_dir: &Path,
    lib_path: &Path,
    file_stem: &str,
    source: &str,
) -> Result<PathBuf> {
    let out_path = crate_src_dir.join(format!("{file_stem}.rs"));
    std::fs::write(&out_path, source).with_context(|| format!("writing {}", out_path.display()))?;

    let lib_src = std::fs::read_to_string(lib_path)
        .with_context(|| format!("reading {}", lib_path.display()))?;
    let marker = format!("mod {file_stem};");
    if !lib_src.contains(&marker) {
        let mut updated = lib_src;
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push('\n');
        updated.push_str("// Ply-generated module declaration -- do not edit this line.\n");
        updated.push_str(&marker);
        updated.push('\n');
        std::fs::write(lib_path, updated)
            .with_context(|| format!("writing {}", lib_path.display()))?;
    }
    Ok(out_path)
}

/// Every declared state type's real fields, keyed by the component's
/// qualified path (`pricing.curves`), in declaration order.
pub type StateFieldIndex = std::collections::BTreeMap<String, Vec<StateField>>;

/// Every crate under `root` with source of its own, keyed by the module
/// path its code is reached by -- `ply_core` for `crates/ply-core`.
///
/// A binary-only crate counts. Its modules are real code that a document
/// can anchor a component at, and refusing to look at them would mean a
/// command-line crate could never say what it holds -- for no better reason
/// than that its crate root is `main.rs`.
///
/// **How the key is decided, and where that is an approximation.** The name
/// comes from `name = "..."` in the crate's `Cargo.toml`, with dashes
/// turned into underscores the way Cargo itself does. A crate that renames
/// its library with an explicit `[lib] name = "..."` different from its
/// package name is read as its package name and would be keyed wrongly;
/// nothing in this workspace does that, and the cost of getting it wrong is
/// one unresolved state line reported as unchecked rather than a false
/// claim of correctness.
///
/// Directories are walked to a depth of three below `root`, which covers
/// the layouts Cargo workspaces actually use (`crates/*`, `tools/*`,
/// `tests/*`) without reading a `[workspace] members` list this crate has
/// no TOML parser for. `target/` and hidden directories are skipped: a
/// build directory contains vendored sources whose crates are not this
/// workspace's, and walking one is slow enough to be noticeable.
pub fn workspace_library_crates(root: &Path) -> std::collections::BTreeMap<String, PathBuf> {
    fn package_name(crate_dir: &Path) -> Option<String> {
        let text = std::fs::read_to_string(crate_dir.join("Cargo.toml")).ok()?;
        let mut in_package = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_package = line == "[package]";
                continue;
            }
            if !in_package {
                continue;
            }
            if let Some(rest) = line.strip_prefix("name") {
                let rest = rest.trim_start().strip_prefix('=')?.trim();
                let name = rest.trim_matches(|c| c == '"' || c == '\'');
                if !name.is_empty() {
                    return Some(name.replace('-', "_"));
                }
            }
        }
        None
    }

    fn walk(dir: &Path, depth: usize, out: &mut std::collections::BTreeMap<String, PathBuf>) {
        if (dir.join("src/lib.rs").exists() || dir.join("src/main.rs").exists())
            && let Some(name) = package_name(dir)
        {
            out.entry(name).or_insert_with(|| dir.to_path_buf());
        }
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skip = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_none_or(|n| n == "target" || n.starts_with('.'));
            if skip {
                continue;
            }
            walk(&path, depth - 1, out);
        }
    }

    let mut out = std::collections::BTreeMap::new();
    walk(root, 3, &mut out);
    out
}

/// Reads every component's declared `state:` type out of real source.
///
/// `root` is the directory the document sits in. A component's type is
/// looked for in the crate its anchor names -- so a workspace-root document
/// whose components are anchored at `ply_core`, `ply_cli` and so on
/// resolves each one against the crate that actually holds it, rather than
/// against a library the workspace root does not have.
///
/// A component whose type cannot be resolved is simply absent from the
/// result. That is the honest answer for the renderer, which then draws the
/// type name alone; `cargo ply check` is the half that turns the same
/// absence into a message (`A0414`, or `W0413` when there was nothing to
/// read at all).
pub fn resolve_state_fields(root: &Path, doc: &crate::model::Document) -> StateFieldIndex {
    // A document with no `state:` anywhere gets no directory walk at all.
    // Finding the crates means reading every `Cargo.toml` under the root,
    // which is real work to do on every render of a document that has
    // nothing to resolve -- and most documents have nothing to resolve.
    if !any_state(&doc.components) {
        return StateFieldIndex::new();
    }
    // An empty path is what `Path::parent` gives for a bare filename, and
    // it names nothing -- every lookup under it fails silently, so a
    // document rendered as `ply.yaml` rather than `./ply.yaml` would draw
    // no shapes and give no reason. Measured, not guessed: that is exactly
    // what happened the first time this ran.
    let root = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    let crates = workspace_library_crates(root);
    let mut out = StateFieldIndex::new();
    walk_state(&doc.components, "", root, &crates, &mut out);
    out
}

/// Whether any component anywhere in the tree declares a `state:`.
fn any_state(components: &indexmap::IndexMap<String, crate::model::Component>) -> bool {
    components
        .values()
        .any(|c| c.state.is_some() || any_state(&c.components))
}

fn walk_state(
    components: &indexmap::IndexMap<String, crate::model::Component>,
    prefix: &str,
    root: &Path,
    crates: &std::collections::BTreeMap<String, PathBuf>,
    out: &mut StateFieldIndex,
) {
    for (name, comp) in components {
        let qualified = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(state) = &comp.state
            && let Some((dir, under)) = crate_and_module_for_anchor(&comp.anchor, root, crates)
            && let Some(fields) = scan_type_fields_under(&dir, &under, &state.of)
        {
            out.insert(qualified.clone(), fields);
        }
        walk_state(&comp.components, &qualified, root, crates, out);
    }
}

/// A crate's own module name, read from its `Cargo.toml` package name with
/// dashes underscored -- the name its code is reached by from outside.
pub fn crate_own_name(crate_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(crate_dir.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = line.strip_prefix("name") {
            let rest = rest.trim_start().strip_prefix('=')?.trim();
            let name = rest.trim_matches(|c| c == '"' || c == '\'');
            if !name.is_empty() {
                return Some(name.replace('-', "_"));
            }
        }
    }
    None
}

/// The crate directory a component's code lives in. The anchor's first
/// segment is a crate name when the document spans a workspace
/// (`ply_core::visual`); in a single-crate document it is a module of the
/// crate the document sits in (`ingest::book`), so `root` is the answer
/// whenever it has a library of its own.
fn crate_and_module_for_anchor(
    anchor: &str,
    root: &Path,
    crates: &std::collections::BTreeMap<String, PathBuf>,
) -> Option<(PathBuf, Vec<String>)> {
    let mut segments: Vec<String> = anchor.split("::").map(str::to_string).collect();
    let head = segments.first().cloned().unwrap_or_default();
    // In a workspace document the anchor's first segment names the crate
    // (`ply_core::visual`) and the rest is a module path inside it. In a
    // single-crate document it is usually a plain module path
    // (`ingest::book`) -- but it may *also* name the crate itself
    // (`crates/ply-core/ply.yaml` anchors its one component at `ply_core`),
    // and treating that as a module would look for a `ply_core` module
    // inside `ply_core`. So the crate's own name is stripped wherever it
    // leads.
    if root.join("src/lib.rs").exists() || root.join("src/main.rs").exists() {
        if crate_own_name(root).as_deref() == Some(head.as_str()) {
            segments.remove(0);
        }
        return Some((root.to_path_buf(), segments));
    }
    let dir = crates.get(&head)?.clone();
    segments.remove(0);
    Some((dir, segments))
}

#[cfg(test)]
mod tests {
    /// `state:` names a type and the fields worth drawing, and the whole
    /// point of the grammar is that a document cannot lie about them: the
    /// fields come from the real source, and a name nobody declared is a
    /// finding rather than a picture. This is the scan that reads them.
    ///
    /// It deliberately reads **every** field, not the ones Ply can build
    /// values of: a private field, or one of a type Ply has no strategy for,
    /// is exactly what a reader most needs shown -- an unbuildable field is
    /// usually why the component's functions report unsupported.
    #[test]
    fn a_types_real_fields_are_read_whatever_their_visibility() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            r#"
use std::collections::BTreeMap;
pub struct OrderBook {
    pub bids: BTreeMap<u64, u32>,
    ticks: Vec<u64>,
    pub clock: std::time::SystemTime,
}
"#,
        )
        .unwrap();

        let fields = scan_type_fields(dir.path(), "OrderBook")
            .expect("the crate declares `OrderBook`, so its fields are readable");
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["bids", "ticks", "clock"],
            "every declared field is read, in declaration order -- a private one is still \
             something the component holds"
        );
        assert!(
            matches!(fields[0].ty, RustType::BTreeMap(_, _)),
            "a map must be recognised as a map so it can be drawn as one: {:?}",
            fields[0].ty
        );
        assert!(
            !fields[2].ty.is_fuzz_supported(),
            "a clock is not a shape Ply builds values of, and that is the fact worth drawing"
        );
    }

    /// A component's state is "resolved under its own anchor, never guessed
    /// at" -- the spec says so and `A0414`'s own message repeats it to the
    /// user. So a type declared somewhere else in the same crate is not
    /// this component's state, and saying it is must fail.
    ///
    /// This is the failure mode one level down from the one `state:` was
    /// built for. The type exists, so a crate-wide scan finds it and passes;
    /// what is wrong is the *attribution*, and a document that misfiles a
    /// type reads exactly like one that does not. Measured before this rule
    /// existed: `ply_core::kernel` claiming a type declared in
    /// `ply_core::diag` passed with exit 0.
    #[test]
    fn a_type_declared_outside_the_anchors_module_is_not_this_components_state() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("kernel")).unwrap();
        std::fs::write(src.join("lib.rs"), "pub mod kernel;\npub mod diag;\n").unwrap();
        std::fs::write(
            src.join("kernel/mod.rs"),
            "pub struct Verdict { pub level: u8 }",
        )
        .unwrap();
        std::fs::write(
            src.join("diag.rs"),
            "pub struct Envelope { pub command: String }",
        )
        .unwrap();

        assert!(
            scan_type_fields_under(dir.path(), &["kernel".to_string()], "Verdict").is_some(),
            "a type declared in the anchored module is this component's state"
        );
        assert!(
            scan_type_fields_under(dir.path(), &["kernel".to_string()], "Envelope").is_none(),
            "`Envelope` is declared in `diag`, not under `kernel`, so a component anchored \
             at `kernel` must not be allowed to claim it"
        );
        assert!(
            scan_type_fields_under(dir.path(), &[], "Envelope").is_some(),
            "a component anchored at the crate root may claim anything in the crate"
        );
    }

    /// The drawn row says what the field is, so the text beside the glyph
    /// has to be what the source actually says. `Vec` where the code says
    /// `Vec<Diagnostic>` is not a shorter true answer -- it drops the only
    /// part of that line a reader was looking for, which is what is *in*
    /// the list.
    #[test]
    fn a_fields_type_reads_the_way_the_source_spells_it() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            r#"
use std::collections::BTreeMap;
pub struct Envelope {
    pub diagnostics: Vec<Diagnostic>,
    pub index: BTreeMap<String, Node>,
    pub next: Option<Box<Envelope>>,
    pub depth: u32,
    pub raw: [u8; 4],
}
"#,
        )
        .unwrap();

        let fields = scan_type_fields(dir.path(), "Envelope").expect("`Envelope` is declared");
        let written: Vec<&str> = fields.iter().map(|f| f.rendered.as_str()).collect();
        assert_eq!(
            written,
            vec![
                "Vec<Diagnostic>",
                "BTreeMap<String, Node>",
                "Option<Box<Envelope>>",
                "u32",
                "[u8; 4]",
            ],
            "each row must carry the type as it is written in the code"
        );
    }

    /// A type the crate does not declare is not a picture with a missing
    /// row -- it is the document making a claim about code that is not
    /// there, which is the failure `state:` exists to make impossible.
    #[test]
    fn a_type_the_crate_does_not_declare_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "pub struct Real { pub a: u8 }").unwrap();
        assert!(
            scan_type_fields(dir.path(), "Invented").is_none(),
            "a type nobody declared must come back as not found, never as an empty shape"
        );
    }

    use super::*;

    /// Contract text is quoted verbatim into diagnostics and into the
    /// rendered cex test's own failure message, so it has to read like the
    /// line the user wrote. Token-stream text spaces out every token, and
    /// the tidier only knew about `|` and `*` -- any contract calling a
    /// method came out as `xs . len () as u32` (2026-08-24 M4 review, D7's
    /// side observation, seen for real on the `BTreeSet` witness path).
    #[test]
    fn contract_text_reads_like_the_line_the_user_wrote_even_with_method_calls() {
        assert_eq!(
            tidy_contract_text("| result | * result == xs . len () as u32"),
            "|result|*result == xs.len() as u32"
        );
        // The M3 shapes stay exactly as they were.
        assert_eq!(
            tidy_contract_text("| result | * result >= lo"),
            "|result|*result >= lo"
        );
    }

    fn write_src(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("lib.rs");
        std::fs::write(&path, content).unwrap();
        path
    }

    // -- anchor resolution follows the crate's own structure (2026-08-25)
    //
    // Ply's two halves disagreed about where a function is: call
    // classification walked `use` imports, inline `mod`s and file modules,
    // anchor resolution read one file's top-level items. So a promise could
    // not be attached to the very callee Ply had just named as unvouched
    // for. These pin the walk that closed that, and the one case that
    // legitimately stays closed.

    /// Lays out `<crate>/src/...` so file modules resolve the way they do
    /// in a real crate, and returns the `src/lib.rs` path.
    fn write_crate(dir: &Path, files: &[(&str, &str)]) -> PathBuf {
        let src = dir.join("src");
        for (rel, content) in files {
            let path = src.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
        }
        src.join("lib.rs")
    }

    #[test]
    fn a_fn_in_an_inline_module_resolves_and_reports_where_it_lives() {
        let dir = tempfile::tempdir().unwrap();
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub mod rates {
    #[ply::ensures(|result| *result <= 10_000)]
    pub fn legacy_rate(tier: u8) -> u32 { if tier == 0 { 150 } else { 90 } }
}
"#,
            )],
        );
        let cf = discover_fn(&lib, "rates::legacy_rate").unwrap();
        assert_eq!(cf.name, "legacy_rate");
        assert_eq!(
            cf.path, "rates::legacy_rate",
            "generated code sits at the crate root, so it must call the function by where it              lives, not by its bare name"
        );
        assert_eq!(cf.ident(), "rates_legacy_rate");
        assert!(cf.ensures.is_some());
    }

    #[test]
    fn a_fn_in_a_file_module_resolves_through_both_of_rusts_spellings() {
        for (rel, name) in [("rates.rs", "rates.rs"), ("rates/mod.rs", "rates/mod.rs")] {
            let dir = tempfile::tempdir().unwrap();
            let lib = write_crate(
                dir.path(),
                &[
                    (
                        "lib.rs",
                        "mod rates;
use rates::legacy_rate;
",
                    ),
                    (
                        rel,
                        "pub fn legacy_rate(tier: u8) -> u32 { tier as u32 }
",
                    ),
                ],
            );
            let cf =
                discover_fn(&lib, "rates::legacy_rate").unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(cf.path, "rates::legacy_rate", "{name}");
        }
    }

    #[test]
    fn a_claim_written_the_way_the_caller_spells_it_lands_on_the_same_fn() {
        // `use rates::legacy_rate;` in lib.rs, and a claim keyed on the
        // bare name. Both spellings must name one function, and both must
        // canonicalise to the same path -- that is what lets a promise
        // written in ply.yaml attach to the callee at a call site.
        let dir = tempfile::tempdir().unwrap();
        let lib = write_crate(
            dir.path(),
            &[
                (
                    "lib.rs",
                    "mod rates;
use rates::legacy_rate;
",
                ),
                (
                    "rates.rs",
                    "pub fn legacy_rate(tier: u8) -> u32 { tier as u32 }
",
                ),
            ],
        );
        assert_eq!(
            discover_fn(&lib, "legacy_rate").unwrap().path,
            discover_fn(&lib, "rates::legacy_rate").unwrap().path
        );
    }

    #[test]
    fn a_fn_in_a_nested_module_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let lib = write_crate(
            dir.path(),
            &[
                (
                    "lib.rs",
                    "mod pricing;
",
                ),
                (
                    "pricing.rs",
                    "pub mod caps { pub fn cap_bps(b: u32) -> u32 { b.min(10_000) } }
",
                ),
            ],
        );
        let cf = discover_fn(&lib, "pricing::caps::cap_bps").unwrap();
        assert_eq!(cf.path, "pricing::caps::cap_bps");
        assert_eq!(cf.ident(), "pricing_caps_cap_bps");
    }

    #[test]
    fn a_private_fn_below_the_crate_root_is_refused_and_says_why() {
        // The one case that stays closed, and it is not a limitation of the
        // walk: the module Ply generates is a sibling of `rates`, so a
        // private item inside `rates` is a name it cannot write. Reported
        // rather than left to surface as a compile error in generated code.
        let dir = tempfile::tempdir().unwrap();
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                "pub mod rates { fn legacy_rate(t: u8) -> u32 { t as u32 } }
",
            )],
        );
        let err = discover_fn(&lib, "rates::legacy_rate")
            .expect_err("a private fn is found but not usable")
            .to_string();
        assert!(err.contains("E0301"), "{err}");
        assert!(
            err.contains("private"),
            "the reason must be the actual one -- not `no such function`: {err}"
        );
    }

    #[test]
    fn a_private_module_makes_everything_inside_it_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                "pub mod a { mod b { pub fn f(x: u32) -> u32 { x } } }
",
            )],
        );
        let err = discover_fn(&lib, "a::b::f")
            .expect_err("`b` is private to `a`")
            .to_string();
        assert!(err.contains("private"), "{err}");
    }

    #[test]
    fn the_item_index_lists_functions_inside_modules_too() {
        // `E0301`'s nearest-name suggestions come from this index, and a
        // suggestion naming something anchor resolution would then refuse
        // is worse than no suggestion -- so the two sets must be the same.
        let dir = tempfile::tempdir().unwrap();
        let lib = write_crate(
            dir.path(),
            &[
                (
                    "lib.rs",
                    "mod rates;
pub fn tiered_fee(x: u32) -> u32 { x }
",
                ),
                (
                    "rates.rs",
                    "pub fn legacy_rate(t: u8) -> u32 { t as u32 }
pub mod caps { pub fn cap(x: u32) -> u32 { x } }
",
                ),
            ],
        );
        let mut index = crate_fn_paths(&lib).unwrap();
        index.sort();
        assert_eq!(
            index,
            vec!["rates::caps::cap", "rates::legacy_rate", "tiered_fee"]
        );
    }

    // -- M4: the fuzz-vs-bounded routing gate --------------------------
    //
    // These pin the exact asymmetry the M4 default-check routing depends
    // on: BTreeSet is fuzz-supported but never bounded-supported (it is
    // §5.4b's own measured Kani exclusion), and a general `Vec<T>` (T != u8)
    // is fuzz-only because the Kani codegen here only ever builds `VecU8`.

    #[test]
    fn btree_set_of_scalar_is_fuzz_supported_but_not_bounded_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
use std::collections::BTreeSet;
#[ply::ensures(|result| *result == xs.len() as u32)]
pub fn count(xs: &BTreeSet<u8>) -> u32 { xs.len() as u32 }
"#,
        );
        let cf = discover_fn(&path, "count").unwrap();
        assert_eq!(cf.params[0].ty, RustType::BTreeSet(Box::new(RustType::U8)));
        assert!(
            cf.is_fuzz_supported(),
            "BTreeSet<u8> must be fuzzable -- proptest has no trouble with it"
        );
        assert!(
            !cf.is_bounded_supported(),
            "BTreeSet must stay Kani-unsupported: §5.4b measured it intractable past one element"
        );
    }

    #[test]
    fn vec_of_non_u8_scalar_is_fuzz_supported_but_not_bounded_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result == xs.len() as u32)]
pub fn count(xs: &Vec<i32>) -> u32 { xs.len() as u32 }
"#,
        );
        let cf = discover_fn(&path, "count").unwrap();
        assert_eq!(cf.params[0].ty, RustType::Vec(Box::new(RustType::I32)));
        assert!(cf.is_fuzz_supported());
        assert!(
            !cf.is_bounded_supported(),
            "this slice's Kani codegen only ever builds VecU8, never a general Vec<T>"
        );
    }

    #[test]
    fn vec_u8_is_both_bounded_and_fuzz_supported_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result <= 255u32 * v.len() as u32)]
pub fn vec_sum(v: &Vec<u8>) -> u32 { 0 }
"#,
        );
        let cf = discover_fn(&path, "vec_sum").unwrap();
        assert_eq!(
            cf.params[0].ty,
            RustType::VecU8,
            "M3's VecU8 shape must not regress to Vec(U8)"
        );
        assert!(cf.is_bounded_supported());
        assert!(cf.is_fuzz_supported());
    }

    // -- 2026-08-25: the fragment widened to §5.4b's own list ------------
    //
    // Until this landed, `rust_type_from_syn` had no `Type::Array` arm and
    // no alias resolution, and knew nothing of `char`, `Option` or
    // `Result` -- so §5.4b's *preferred* bounded shape came back
    // `Unsupported("[u32 ; 4]")` and `type AccountId = u64` moved a
    // function out of the checkable set (vetting 004 finding 5). Costs
    // measured, not assumed: each shape verifies in 0.03-0.06s of Kani
    // time on a trivial body (docs/post-004-fixes.md).

    #[test]
    fn a_fixed_size_array_is_the_preferred_bounded_shape_not_an_unsupported_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::requires(amount_cents <= 100_000_000 && tier < 4)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn carded_fee_cents(amount_cents: u32, tier: u8, card_bps: [u32; 4]) -> u32 { 0 }
"#,
        );
        let cf = discover_fn(&path, "carded_fee_cents").unwrap();
        assert_eq!(
            cf.params[2].ty,
            RustType::Array(Box::new(RustType::U32), 4),
            "§5.4b calls a fixed-size array v1's preferred bounded shape"
        );
        assert!(cf.is_bounded_supported());
        assert!(cf.is_fuzz_supported());
        let harness_out = generate_proof_module(&cf, 2, &[]).unwrap();
        assert!(
            harness_out
                .module_source
                .contains("let card_bps: [u32; 4] = kani::any();"),
            "{}",
            harness_out.module_source
        );
        assert!(
            harness_out.unwind.is_none(),
            "an array's length is a compile-time constant -- no unwind annotation, unlike `Vec`"
        );
    }

    #[test]
    fn char_option_and_result_are_in_the_fragment() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result >= 0)]
pub fn classify(c: char, hint: Option<u32>, parsed: Result<u32, u8>) -> i32 { 0 }
"#,
        );
        let cf = discover_fn(&path, "classify").unwrap();
        assert_eq!(cf.params[0].ty, RustType::Char);
        assert_eq!(cf.params[1].ty, RustType::Option(Box::new(RustType::U32)));
        assert_eq!(
            cf.params[2].ty,
            RustType::Result(Box::new(RustType::U32), Box::new(RustType::U8))
        );
        assert!(
            cf.is_bounded_supported(),
            "§5.4b lists all three as cheap unconditionally"
        );
    }

    #[test]
    fn a_type_alias_resolves_to_what_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
pub type AccountId = u64;
pub type Bps = u32;
#[ply::ensures(|result| *result >= 0)]
pub fn owed(account: AccountId, rate: Bps) -> i64 { 0 }
"#,
        );
        let cf = discover_fn(&path, "owed").unwrap();
        assert_eq!(
            cf.params[0].ty,
            RustType::U64,
            "an alias is transparent in Rust, and one line of it must not move a fn out of the \
             checkable set (vetting 004 finding 5)"
        );
        assert_eq!(cf.params[1].ty, RustType::U32);
        assert!(cf.is_bounded_supported());
    }

    // -- 2026-08-27: usize/isize, the `NonZero` family, and `Duration` -----
    //
    // The rate-limiter measurement (docs/greenfield-ratelimiter-design.md's
    // Flowgate fixture) found these dominate ordinary Rust's public surface
    // far more than any shape already in the fragment: `Duration` (20 uses),
    // `NonZeroU32` (19), `NonZeroUsize` (8), `usize` (3) against 82 total
    // type uses, of which Ply supported 17 (21%) before this landed.

    #[test]
    fn usize_and_isize_are_bounded_and_fuzz_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result >= 0)]
pub fn f(len: usize, delta: isize) -> i64 { 0 }
"#,
        );
        let cf = discover_fn(&path, "f").unwrap();
        assert_eq!(cf.params[0].ty, RustType::Usize);
        assert_eq!(cf.params[1].ty, RustType::Isize);
        assert!(
            cf.is_bounded_supported(),
            "usize/isize are pointer-width integers -- exactly the shape §5.4b already calls \
             cheap unconditionally for every other integer width"
        );
        assert!(cf.is_fuzz_supported());
    }

    #[test]
    fn nonzero_u32_is_bounded_and_fuzz_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
use std::num::NonZeroU32;
#[ply::ensures(|result| *result > 0)]
pub fn check_n(n: NonZeroU32) -> u32 { n.get() }
"#,
        );
        let cf = discover_fn(&path, "check_n").unwrap();
        assert_eq!(cf.params[0].ty, RustType::NonZero(Box::new(RustType::U32)));
        assert!(
            cf.is_bounded_supported(),
            "NonZeroU32 must be a shape Ply's Kani codegen can build, not refused by name"
        );
        assert!(cf.is_fuzz_supported());
    }

    #[test]
    fn nonzero_usize_is_bounded_and_fuzz_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
use std::num::NonZeroUsize;
#[ply::ensures(|result| *result > 0)]
pub fn cap(n: NonZeroUsize) -> usize { n.get() }
"#,
        );
        let cf = discover_fn(&path, "cap").unwrap();
        assert_eq!(
            cf.params[0].ty,
            RustType::NonZero(Box::new(RustType::Usize))
        );
        assert!(cf.is_bounded_supported());
        assert!(cf.is_fuzz_supported());
    }

    #[test]
    fn duration_is_bounded_and_fuzz_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
use std::time::Duration;
#[ply::ensures(|result| result.as_nanos() >= 0)]
pub fn identity(d: Duration) -> Duration { d }
"#,
        );
        let cf = discover_fn(&path, "identity").unwrap();
        assert_eq!(cf.params[0].ty, RustType::Duration);
        assert!(
            cf.is_bounded_supported(),
            "Duration must be a shape Ply's Kani codegen can build -- a pair of integers, one \
             bounded to under one billion, not a struct with private fields Ply must derive \
             Arbitrary for"
        );
        assert!(cf.is_fuzz_supported());
    }

    #[test]
    fn duration_proof_never_lets_nanos_reach_a_billion() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
use std::time::Duration;
#[ply::ensures(|result| result.as_nanos() >= 0)]
pub fn identity(d: Duration) -> Duration { d }
"#,
        );
        let cf = discover_fn(&path, "identity").unwrap();
        let harness_out = generate_proof_module(&cf, 2, &[]).unwrap();
        assert!(
            harness_out
                .module_source
                .contains("kani::assume(d_nanos < 1_000_000_000"),
            "a generated Duration must never let its own construction produce nanos >= 1e9 -- \
             the standard library never returns such a value, and a proof that could see one \
             would be exploring a state the type forbids:\n{}",
            harness_out.module_source
        );
        assert!(
            harness_out
                .module_source
                .contains("Duration::new(d_secs, d_nanos)"),
            "{}",
            harness_out.module_source
        );
    }

    #[test]
    fn nonzero_proof_never_lets_the_inner_value_be_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
use std::num::NonZeroU32;
#[ply::ensures(|result| *result > 0)]
pub fn check_n(n: NonZeroU32) -> u32 { n.get() }
"#,
        );
        let cf = discover_fn(&path, "check_n").unwrap();
        let harness_out = generate_proof_module(&cf, 2, &[]).unwrap();
        assert!(
            harness_out
                .module_source
                .contains("kani::assume(n_inner != 0")
                || harness_out
                    .module_source
                    .contains("kani::assume(n_inner != 0u32"),
            "a generated NonZeroU32 must never let its own inner value be zero -- a proof that \
             could see zero would be exploring a state the type forbids, and the witness it \
             produced could never occur for real:\n{}",
            harness_out.module_source
        );
        assert!(
            harness_out
                .module_source
                .contains("NonZeroU32::new(n_inner).unwrap()"),
            "{}",
            harness_out.module_source
        );
    }

    #[test]
    fn nonzero_and_duration_are_full_domain_since_construction_is_unbounded() {
        // D5's containment argument (§5.5): standing on a callee's
        // `bounded(k)` proof costs nothing only when that proof already
        // covers whatever a caller could ever pass. `NonZero`'s inner
        // integer ranges over its *entire* type (only the assume rules out
        // zero, which is not a value a real NonZeroU32 could hold anyway),
        // and `Duration`'s seconds field is never bounded at all -- so both
        // must be `is_full_domain() == true`, same as any plain scalar,
        // unlike `VecU8` (bounded to length `k`).
        assert!(RustType::NonZero(Box::new(RustType::U32)).is_full_domain());
        assert!(RustType::NonZero(Box::new(RustType::Usize)).is_full_domain());
        assert!(RustType::Duration.is_full_domain());
    }

    // -- the sampling/proving split (task, 2026-08-27): floats are the
    // headline case -- fuzz-supported, never bounded-supported, by
    // deliberate design decision rather than a measured Kani exclusion.

    #[test]
    fn f32_and_f64_are_fuzz_supported_but_never_bounded_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result >= x)]
pub fn increment(x: f64) -> f64 { x + 1.0 }
#[ply::ensures(|result| *result == y)]
pub fn identity32(y: f32) -> f32 { y }
"#,
        );
        let inc = discover_fn(&path, "increment").unwrap();
        assert_eq!(inc.params[0].ty, RustType::F64);
        assert_eq!(inc.return_type, RustType::F64);
        assert!(
            inc.is_fuzz_supported(),
            "a plain f64 parameter must be sampleable -- proptest builds one as cheaply as any \
             other scalar"
        );
        assert!(
            !inc.is_bounded_supported(),
            "f64 must stay refused on the proving engine -- this is the split's own headline \
             case, a deliberate decision (§ split, not a measured exclusion the way BTreeSet is)"
        );

        let id32 = discover_fn(&path, "identity32").unwrap();
        assert_eq!(id32.params[0].ty, RustType::F32);
        assert!(id32.is_fuzz_supported());
        assert!(!id32.is_bounded_supported());
    }

    #[test]
    fn a_float_typed_fn_is_flagged_by_has_float_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result >= x)]
pub fn increment(x: f64) -> f64 { x + 1.0 }
#[ply::ensures(|result| *result == x)]
pub fn scalar(x: u32) -> u32 { x }
"#,
        );
        assert!(discover_fn(&path, "increment").unwrap().has_float_shape());
        assert!(!discover_fn(&path, "scalar").unwrap().has_float_shape());
    }

    #[test]
    fn instant_stays_refused_by_name_struct_and_enum_no_longer_do() {
        // `Instant` is explicitly out of scope, permanently rather than
        // pending: a monotonic clock the harness cannot rewind or fake
        // needs its own design, unrelated to struct/enum parameters. This
        // must not regress.
        //
        // `String` used to sit in this same list -- it no longer does (task,
        // 2026-08-27, the sampling/proving split's second headline case):
        // see `string_is_fuzz_supported_but_never_bounded_supported` below
        // for its own, now-supported, behaviour.
        //
        // `Foo`/`Bar` used to stay `Unsupported` too (struct/enum
        // parameters were out of scope) -- this task changes that, but only
        // through the enrichment pass `verify` runs after `discover_fn`
        // (`enrich_contract_fn_user_types`), never inside `discover_fn`
        // itself (used directly by D5's same-crate-callee path, which this
        // task does not touch). So `discover_fn` alone still reports both
        // `Unsupported` here, honestly reflecting what it does on its own;
        // the second half of this test proves enrichment resolves them both.
        // `write_crate` (not `write_src`): enrichment scans `<crate_dir>/src/`
        // for struct/enum declarations, so this test needs the real
        // crate-directory layout that convention assumes, not `write_src`'s
        // single loose file (fine for `discover_fn` alone, which just reads
        // one path, but not for `enrich_contract_fn_user_types`'s own
        // crate-wide scan).
        let dir = tempfile::tempdir().unwrap();
        let path = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
use std::time::Instant;
#[derive(Clone, Copy)]
pub struct Foo { pub x: u32 }
pub enum Bar { A, B }
#[ply::ensures(|result| *result >= 0)]
pub fn f(t: Instant, foo: Foo, bar: Bar) -> i64 { 0 }
"#,
            )],
        );
        let mut cf = discover_fn(&path, "f").unwrap();
        assert!(
            matches!(cf.params[0].ty, RustType::Unsupported(_)),
            "Instant: {:?}",
            cf.params[0].ty
        );
        assert!(
            matches!(cf.params[1].ty, RustType::Unsupported(_)),
            "struct, before enrichment: {:?}",
            cf.params[1].ty
        );
        assert!(
            matches!(cf.params[2].ty, RustType::Unsupported(_)),
            "enum, before enrichment: {:?}",
            cf.params[2].ty
        );

        enrich_contract_fn_user_types(&mut cf, dir.path(), &RouteTable::new());
        assert!(
            matches!(cf.params[0].ty, RustType::Unsupported(_)),
            "Instant must still be unsupported after enrichment -- it is not a struct/enum this \
             crate declares: {:?}",
            cf.params[0].ty
        );
        assert!(
            matches!(cf.params[1].ty, RustType::UserTypeFields(_)),
            "Foo's fields are all public -- enrichment must resolve it: {:?}",
            cf.params[1].ty
        );
        assert!(
            matches!(cf.params[2].ty, RustType::UserTypeFields(_)),
            "Bar's variants carry no fields at all, so nothing is private -- enrichment must \
             resolve it too: {:?}",
            cf.params[2].ty
        );
        assert!(!cf.is_bounded_supported());
        assert!(!cf.is_fuzz_supported());
    }

    // -- the sampling/proving split's second headline case (task,
    // 2026-08-27): `String` is fuzz-supported, never bounded-supported, the
    // same asymmetry as `f32`/`f64` above.

    #[test]
    fn string_is_fuzz_supported_but_never_bounded_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result == old(s).len())]
pub fn byte_len(s: String) -> usize { s.len() }
"#,
        );
        let cf = discover_fn(&path, "byte_len").unwrap();
        assert_eq!(cf.params[0].ty, RustType::String);
        assert!(
            cf.is_fuzz_supported(),
            "a plain String parameter must be sampleable -- proptest builds one as cheaply as \
             any other scalar shape"
        );
        assert!(
            !cf.is_bounded_supported(),
            "String must stay refused on the proving engine -- the split's own second headline \
             case, a deliberate decision, not a measured Kani exclusion"
        );
    }

    #[test]
    fn a_string_typed_fn_is_flagged_by_has_string_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result == old(s).len())]
pub fn byte_len(s: String) -> usize { s.len() }
#[ply::ensures(|result| *result == x)]
pub fn scalar(x: u32) -> u32 { x }
"#,
        );
        assert!(discover_fn(&path, "byte_len").unwrap().has_string_shape());
        assert!(!discover_fn(&path, "scalar").unwrap().has_string_shape());
    }

    #[test]
    fn nested_string_is_fuzz_supported_but_never_silently_bounded() {
        // Composition (2026-09-02, TODO.md) is what makes `Option<String>`
        // buildable at all now -- the defect this task measures and fixes.
        // What this test still pins, and must always pin, is the standing
        // obligation composition is not allowed to touch: nesting a
        // `String` must never make the *proof* engine's own answer widen,
        // even though the sampling engine's answer correctly flips to
        // `true`.
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| result.is_none() || result.as_ref().unwrap().len() >= 0)]
pub fn f(s: Option<String>) -> Option<String> { s }
"#,
        );
        let cf = discover_fn(&path, "f").unwrap();
        assert_eq!(
            cf.params[0].ty,
            RustType::Option(Box::new(RustType::String))
        );
        assert!(
            cf.is_fuzz_supported(),
            "a `String` is buildable alone -- nesting it must not refuse it: {:?}",
            cf.params[0].ty
        );
        assert!(
            !cf.is_bounded_supported(),
            "Option<String> must never read as bounded-supported: {:?}",
            cf.params[0].ty
        );
    }

    #[test]
    fn an_array_of_a_shape_kani_cannot_build_is_fuzz_supported_but_never_bounded() {
        // `BTreeSet<u8>` was always fuzz-supported alone; composition
        // (2026-09-02) is what lets it nest inside `[T; 2]` too. What this
        // test still pins is that the *proof* engine's own list does not
        // move: Kani has never built a `BTreeSet` past one element, and
        // nesting one inside an array must not change that.
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
use std::collections::BTreeSet;
#[ply::ensures(|result| *result >= 0)]
pub fn f(x: [BTreeSet<u8>; 2]) -> i32 { 0 }
"#,
        );
        let cf = discover_fn(&path, "f").unwrap();
        assert_eq!(
            cf.params[0].ty,
            RustType::Array(Box::new(RustType::BTreeSet(Box::new(RustType::U8))), 2)
        );
        assert!(
            cf.is_fuzz_supported(),
            "every element (BTreeSet<u8>) is buildable alone: {:?}",
            cf.params[0].ty
        );
        assert!(
            !cf.is_bounded_supported(),
            "widening the fragment must not widen it past what the proof engine builds: {:?}",
            cf.params[0].ty
        );
    }

    #[test]
    fn discovers_clamp_contract() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 {
    x.min(100)
}
"#,
        );
        let cf = discover_fn(&path, "clamp").unwrap();
        assert_eq!(cf.name, "clamp");
        assert_eq!(cf.params.len(), 1);
        assert_eq!(cf.params[0].ty, RustType::U32);
        assert!(cf.ensures.is_some());
        assert!(cf.is_bounded_supported());
    }

    #[test]
    fn discovers_vec_param_by_ref() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result <= 255u32 * v.len() as u32)]
pub fn vec_sum(v: &Vec<u8>) -> u32 { 0 }
"#,
        );
        let cf = discover_fn(&path, "vec_sum").unwrap();
        assert_eq!(cf.params[0].ty, RustType::VecU8);
        assert!(cf.params[0].by_ref);
        assert!(cf.has_vec_param());
    }

    /// A parameter the function can write back through is the one shape
    /// `old()` exists for -- and it is not one either engine can check:
    /// Ply builds every argument itself and hands it in, and §5.4b's
    /// supported list stops at a shared `&T`. Until 2026-08-25 the reader
    /// looked straight through the `&mut` and recorded a plain `u32`, so
    /// codegen produced a harness that passed a shared reference where a
    /// mutable one was wanted. Under the model checker that surfaced as
    /// "Ply's Kani adapter could not interpret Kani's output"; under the
    /// random-input tier as a compiler type error inside Ply's own
    /// generated file. Both are internal errors about Ply, not answers
    /// about the user's function. The shape must be refused by name.
    #[test]
    fn a_parameter_the_function_writes_back_through_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *counter == old(*counter) + 1)]
pub fn bump_in_place(counter: &mut u32) { *counter += 1; }
"#,
        );
        let cf = discover_fn(&path, "bump_in_place").unwrap();
        assert_eq!(
            cf.params[0].ty,
            RustType::Unsupported("&mut u32".to_string()),
            "a `&mut` parameter must be recorded as a shape Ply does not build, spelled the way \
             the user wrote it -- recorded as a plain `u32` it produces a harness that does not \
             compile"
        );
        assert!(
            !cf.is_bounded_supported(),
            "the model-checking codegen cannot build a mutable reference"
        );
        assert!(
            !cf.is_fuzz_supported(),
            "neither can the random-input codegen"
        );
    }

    #[test]
    fn generates_scalar_harness_with_no_unwind() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 { x.min(100) }
"#,
        );
        let cf = discover_fn(&path, "clamp").unwrap();
        let harness_out = generate_proof_module(&cf, 2, &[]).unwrap();
        assert!(
            harness_out.unwind.is_none(),
            "scalar-only fn must not get an unwind annotation"
        );
        assert!(harness_out.module_source.contains("kani::any()"));
        assert!(
            harness_out
                .module_source
                .contains("#[kani::proof_for_contract(clamp)]")
        );
        assert_eq!(harness_out.proof_fn_path, "ply_generated::ply_proof_clamp");
    }

    #[test]
    fn generates_vec_harness_with_measured_unwind() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result <= 255u32 * v.len() as u32)]
pub fn vec_sum(v: &Vec<u8>) -> u32 { 0 }
"#,
        );
        let cf = discover_fn(&path, "vec_sum").unwrap();
        let harness_out = generate_proof_module(&cf, 8, &[]).unwrap();
        assert_eq!(
            harness_out.unwind,
            Some(9),
            "measured bound for N=8 is N+1=9 (see m3-slice-findings.md)"
        );
        assert!(harness_out.module_source.contains("#[kani::unwind(9)]"));
        assert!(
            harness_out
                .module_source
                .contains("kani::vec::any_vec::<u8, 8>()")
        );
        assert!(harness_out.module_source.contains("vec_sum(&v);"));
    }

    #[test]
    fn write_generated_module_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path();
        let lib_path = write_src(src_dir, "pub fn f() {}\n");
        write_generated_module(src_dir, &lib_path, "// one\n").unwrap();
        let after_first = std::fs::read_to_string(&lib_path).unwrap();
        write_generated_module(src_dir, &lib_path, "// two\n").unwrap();
        let after_second = std::fs::read_to_string(&lib_path).unwrap();
        assert_eq!(
            after_first, after_second,
            "mod declaration must be inserted exactly once"
        );
        assert_eq!(
            std::fs::read_to_string(src_dir.join("ply_generated.rs")).unwrap(),
            "// two\n",
            "the generated file's content still updates on rerun"
        );
    }

    // -- method resolution: `is_method`, `call_expr`/`import_path`, and the
    // return-type gate (adversarial review, 2026-08-27) --

    #[test]
    fn a_receiverless_method_is_flagged_is_method_with_a_type_qualified_call_expr() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
pub struct Bucket { cap: u32 }
impl Bucket {
    #[ply::ensures(|result| result.cap == cap)]
    pub fn new(cap: u32) -> Self { Bucket { cap } }
}
"#,
        );
        let cf = discover_fn(&path, "Bucket::new").unwrap();
        assert!(cf.is_method, "a `Type::method` claim must set is_method");
        assert_eq!(
            cf.call_expr(),
            "Bucket::new",
            "generated code must call this by `Type::method`, never a bare `new` (which is not              importable at all -- a method is not a `use`-able item)"
        );
        assert_eq!(
            cf.import_path(),
            "Bucket",
            "generated code must import the *type*, not the method, so `call_expr()` resolves"
        );
        assert_eq!(
            cf.return_type,
            RustType::SelfType,
            "a constructor's `Self` return must never gate whether Ply checks it"
        );
        assert!(
            cf.is_bounded_supported() && cf.is_fuzz_supported(),
            "a `u32` parameter and a `Self` return are both fully supported"
        );
    }

    #[test]
    fn a_free_function_keeps_its_bare_call_expr_and_full_import_path_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
mod rates { pub fn legacy_rate(t: u8) -> u32 { 150 } }
"#,
        );
        let cf = discover_fn(&path, "rates::legacy_rate").unwrap();
        assert!(!cf.is_method);
        assert_eq!(
            cf.call_expr(),
            "legacy_rate",
            "unchanged from before methods existed"
        );
        assert_eq!(
            cf.import_path(),
            "rates::legacy_rate",
            "unchanged -- the whole path is what a free function's own `use` still imports"
        );
    }

    #[test]
    fn a_receiverless_methods_unmodelled_return_type_no_longer_blocks_either_engine() {
        // Retracts this test's own former name and assertions (measured
        // 2026-09-01, docs/reach-measurement-2.md, The-Ply-Spec.md §5.4b):
        // neither engine ever constructs a return value -- the real call
        // produces it -- so refusing a fn over a return type Ply's parser
        // does not model was blocking real evidence for no technical
        // reason. Measured on both engines before this changed (a
        // `std::cmp::Ordering` return, unmodelled the same way `Elsewhere`
        // below is): `fuzzed(64)` and `bounded(2)` on a correct fn, and a
        // real `violation` with a witness on a broken one, on *both*
        // engines independently. The return type's own classification
        // (`RustType::Unsupported`) is unchanged and still correct -- it is
        // only the *gate* that read that classification and refused to
        // check the fn at all that is gone.
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
pub struct Bucket;
pub struct Elsewhere { pub n: u32 }
impl Bucket {
    #[ply::ensures(|result| result.n == 0)]
    pub fn make_elsewhere() -> Elsewhere { Elsewhere { n: 0 } }
}
"#,
        );
        let cf = discover_fn(&path, "Bucket::make_elsewhere").unwrap();
        assert!(
            matches!(cf.return_type, RustType::Unsupported(_)),
            "an ordinary struct Ply's parser does not model is still classified the same way \
             in return position, exactly as it would be as a parameter: {:?}",
            cf.return_type
        );
        assert!(
            cf.is_bounded_supported(),
            "the return type must never gate `bounded` -- Kani never constructs a return value"
        );
        assert!(
            cf.is_fuzz_supported(),
            "the return type must never gate `fuzz` -- proptest never constructs a return value \
             either"
        );
    }

    #[test]
    fn last_two_segments_splits_a_method_path_and_passes_through_a_short_one() {
        assert_eq!(
            last_two_segments("bucket::TokenBucket::new"),
            "TokenBucket::new"
        );
        assert_eq!(last_two_segments("Bucket::new"), "Bucket::new");
        assert_eq!(last_two_segments("legacy_rate"), "legacy_rate");
    }

    // -- receiver construction (docs/review-self-construction.md's "fourth
    // option") -- `discover_method_with_receiver` is the second, narrower
    // path a caller tries only after `callgraph::Resolver` has already
    // refused a `&self` method; these tests call it directly, the same way
    // `verify` will, never through the shared resolver.

    /// The plain success case: a constructible type, a `&self` method with
    /// no other same-shape sibling -- the receiver's pool is exactly the
    /// checked method itself (constructor-only is length 0 of *this* pool,
    /// never an empty one).
    #[test]
    fn a_self_method_on_a_constructible_type_builds_a_receiver_plan() {
        let dir = tempfile::tempdir().unwrap();
        write_crate(
            dir.path(),
            &[(
                "bucket.rs",
                r#"
pub struct Bucket { cap: u32 }
impl Bucket {
    pub fn new(cap: u32) -> Self { Bucket { cap } }
    #[ply::ensures(|result| *result <= 1_000_000)]
    pub fn capacity(&self) -> u32 { self.cap }
}
"#,
            )],
        );
        let cf = discover_method_with_receiver(
            dir.path(),
            "bucket::Bucket::capacity",
            &RouteTable::new(),
        )
        .unwrap();
        assert_eq!(cf.name, "capacity");
        assert!(cf.is_method);
        let plan = cf.receiver.expect("a receiver plan must be attached");
        assert_eq!(plan.type_name, "Bucket");
        assert_eq!(plan.constructor, "Bucket::new");
        assert_eq!(plan.ctor_params.len(), 1);
        assert_eq!(plan.max_sequence_len, MAX_RECEIVER_SEQUENCE_LEN);
        assert_eq!(
            plan.operations.len(),
            1,
            "with no other &self method sharing `capacity`'s (empty) shape, the pool is the \
             checked method alone -- still a real sequence pool, not an absence of one"
        );
        assert_eq!(plan.operations[0].call_path, "Bucket::capacity");
    }

    /// Defect (2026-09-01, docs/reach-measurement-2.md): a parameter written
    /// `&Self` was left `Unsupported("Self")` even though the exact same
    /// type spelled by name (`&Widget`, in a sibling method below) already
    /// resolves through the enrichment pass to a buildable constructor
    /// call. `Self` in parameter position names the enclosing `impl`
    /// block's own type, exactly as the receiver already does, so it must
    /// resolve the same way -- reusing `ReceiverPlan::type_name`, the
    /// receiver's own already-resolved answer, rather than a second lookup
    /// that has to rediscover the same fact.
    #[test]
    fn self_typed_parameter_resolves_like_the_named_type() {
        let dir = tempfile::tempdir().unwrap();
        write_crate(
            dir.path(),
            &[(
                "widget.rs",
                r#"
pub struct Widget { n: u32 }
impl Widget {
    pub fn new(n: u32) -> Self { Widget { n } }
    #[ply::ensures(|result| *result == (self.n == other.n))]
    pub fn same_as_self(&self, other: &Self) -> bool { self.n == other.n }
    #[ply::ensures(|result| *result == (self.n == other.n))]
    pub fn same_as_named(&self, other: &Widget) -> bool { self.n == other.n }
}
"#,
            )],
        );
        let cf_self = discover_method_with_receiver(
            dir.path(),
            "widget::Widget::same_as_self",
            &RouteTable::new(),
        )
        .unwrap();
        let cf_named = discover_method_with_receiver(
            dir.path(),
            "widget::Widget::same_as_named",
            &RouteTable::new(),
        )
        .unwrap();
        assert_eq!(
            cf_named.params[0].ty, cf_self.params[0].ty,
            "a `&Self` parameter must resolve to exactly the same buildable type as the same \
             parameter spelled by its name -- got {:?} for `&Self` vs {:?} for `&Widget`",
            cf_self.params[0].ty, cf_named.params[0].ty,
        );
        assert!(
            matches!(
                cf_self.params[0].ty,
                RustType::UserTypeCtor(_) | RustType::UserTypeFields(_)
            ),
            "a &Self parameter must resolve to the enclosing type, just like &Widget would: {:?}",
            cf_self.params[0].ty
        );
        assert!(cf_self.is_fuzz_supported());
    }

    /// The decisive shape for the sequence feature to mean anything: every
    /// `&self`/`&mut self` sibling operation with a buildable parameter
    /// shape is pooled alongside the checked method, whatever its own
    /// shape is -- widened 2026-08-27 (docs/review-caveats.md N3, "the
    /// twelfth false clean") from an earlier, narrower rule that required a
    /// pooled operation's own parameters to match the checked method's
    /// exactly. That rule is exactly what emptied the pool for an ordinary
    /// Rust type: a `&mut self` mutator (`bump` here) almost never shares
    /// its read-only sibling's parameter list, and neither does a
    /// zero-argument one (`reset`), so nothing that could actually change
    /// the receiver's state ever qualified.
    #[test]
    fn every_buildable_sibling_operation_is_pooled_whatever_its_shape() {
        let dir = tempfile::tempdir().unwrap();
        write_crate(
            dir.path(),
            &[(
                "meter.rs",
                r#"
pub struct Meter { n: std::cell::Cell<u32> }
impl Meter {
    pub fn new() -> Self { Meter { n: std::cell::Cell::new(0) } }
    pub fn bump(&self, amount: u32) -> u32 { self.n.set(self.n.get() + amount); self.n.get() }
    #[ply::ensures(|result| *result < 1_000_000)]
    pub fn spend(&self, amount: u32) -> u32 { self.n.set(self.n.get() - amount); self.n.get() }
    pub fn reset(&self) { self.n.set(0); }
    pub fn set_direct(&mut self, amount: u32) { self.n.set(amount); }
}
"#,
            )],
        );
        let cf =
            discover_method_with_receiver(dir.path(), "meter::Meter::spend", &RouteTable::new())
                .unwrap();
        let plan = cf.receiver.expect("a receiver plan must be attached");
        let call_paths: Vec<&str> = plan
            .operations
            .iter()
            .map(|o| o.call_path.as_str())
            .collect();
        assert!(
            call_paths.contains(&"Meter::spend"),
            "the checked method is always in its own pool: {call_paths:?}"
        );
        assert!(
            call_paths.contains(&"Meter::bump"),
            "`bump(u32)` shares `spend`'s own shape and must be pooled: {call_paths:?}"
        );
        assert!(
            call_paths.contains(&"Meter::reset"),
            "`reset()` takes no parameters -- a *different* shape from `spend(u32)` -- and must \
             still be pooled: a mixed-shape pool is exactly what this task built: {call_paths:?}"
        );
        assert!(
            call_paths.contains(&"Meter::set_direct"),
            "`set_direct` takes `&mut self` -- the ordinary way a Rust type changes state -- and \
             must be pooled too, or nothing in the sequence could ever change the receiver: \
             {call_paths:?}"
        );
        let set_direct_op = plan
            .operations
            .iter()
            .find(|o| o.call_path == "Meter::set_direct")
            .unwrap();
        assert!(
            set_direct_op.takes_mut_self,
            "codegen needs to know this operation borrows `&mut`, not `&`, to call it correctly"
        );
    }

    /// The refusal-by-name half of the feature: a type with genuinely no
    /// constructor is refused, naming the type -- never silently filled in
    /// field-by-field (`docs/review-self-construction.md` rejects that for
    /// exactly this reason).
    #[test]
    fn a_type_with_no_constructor_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        write_crate(
            dir.path(),
            &[(
                "gauge.rs",
                r#"
pub struct Gauge { n: u32 }
impl Gauge {
    #[ply::ensures(|result| *result == self_n())]
    pub fn read(&self) -> u32 { self.n }
}
fn self_n() -> u32 { 0 }
"#,
            )],
        );
        let err =
            discover_method_with_receiver(dir.path(), "gauge::Gauge::read", &RouteTable::new())
                .unwrap_err();
        match &err {
            ReceiverError::NoConstructor { type_name } => assert_eq!(type_name, "Gauge"),
            other => panic!("expected NoConstructor naming `Gauge`, got {other:?}"),
        }
        let msg = err.to_string();
        assert!(
            msg.contains("Gauge"),
            "the refusal must name the type: {msg}"
        );
    }

    /// A constructor exists, but every candidate takes a type Ply's
    /// checkers cannot build -- refused by name, naming *which* type,
    /// never silently skipped to another rule.
    #[test]
    fn a_constructor_needing_an_unsupported_type_is_refused_by_name() {
        // `Instant`, not `String`, is the unsupported type here (changed by
        // the sampling/proving split task, 2026-08-27): `String` is now a
        // sample-supported shape (`RustType::String`), so a constructor
        // parameter of that type no longer demonstrates this refusal --
        // `Instant` still does (a monotonic clock the harness cannot
        // rewind or fake, unchanged and still `Unsupported`, see
        // `instant_struct_and_enum_stay_refused_by_name_unchanged` above).
        let dir = tempfile::tempdir().unwrap();
        write_crate(
            dir.path(),
            &[(
                "labelled.rs",
                r#"
pub struct Labelled { at: std::time::Instant }
impl Labelled {
    pub fn new(at: std::time::Instant) -> Self { Labelled { at } }
    #[ply::ensures(|result| *result == self_label_len())]
    pub fn label_len(&self) -> u32 { self_label_len() }
}
fn self_label_len() -> u32 { 0 }
"#,
            )],
        );
        let err = discover_method_with_receiver(
            dir.path(),
            "labelled::Labelled::label_len",
            &RouteTable::new(),
        )
        .unwrap_err();
        match err {
            ReceiverError::UnsupportedConstructorParam {
                type_name,
                ctor_name,
                bad_type,
            } => {
                assert_eq!(type_name, "Labelled");
                assert_eq!(ctor_name, "Labelled::new");
                assert!(bad_type.contains("Instant"), "{bad_type}");
            }
            other => panic!("expected UnsupportedConstructorParam, got {other:?}"),
        }
    }

    /// A `&mut self` target stays refused exactly as before this task: Ply
    /// still has no way to state what such a call is supposed to change
    /// about the receiver, so building one would not close the gap.
    #[test]
    fn a_mut_self_method_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_crate(
            dir.path(),
            &[(
                "counter.rs",
                r#"
pub struct Counter { n: u32 }
impl Counter {
    pub fn new() -> Self { Counter { n: 0 } }
    pub fn bump(&mut self) { self.n += 1; }
}
"#,
            )],
        );
        let err =
            discover_method_with_receiver(dir.path(), "counter::Counter::bump", &RouteTable::new())
                .unwrap_err();
        assert!(matches!(err, ReceiverError::MutableOrOwnedReceiver));
    }

    /// A trait-impl method is not in any inherent `impl` block this scan
    /// reads -- it must come back `MethodNotFound`, so the caller falls
    /// back to the resolver's own (correct) "defined in a trait
    /// implementation" refusal, rather than this scan inventing its own
    /// wrong answer.
    #[test]
    fn a_trait_impl_method_is_not_found_by_this_scan() {
        let dir = tempfile::tempdir().unwrap();
        write_crate(
            dir.path(),
            &[(
                "widget.rs",
                r#"
pub struct Widget;
impl Widget {
    pub fn new() -> Self { Widget }
}
pub trait Describe { fn describe(&self) -> u32; }
impl Describe for Widget {
    fn describe(&self) -> u32 { 0 }
}
"#,
            )],
        );
        let err = discover_method_with_receiver(
            dir.path(),
            "widget::Widget::describe",
            &RouteTable::new(),
        )
        .unwrap_err();
        assert!(matches!(err, ReceiverError::MethodNotFound));
    }

    /// A claim path nested more than one module segment deep is an honest,
    /// named limit (`ReceiverError::UnsupportedModulePath`), not a silent
    /// wrong answer.
    #[test]
    fn a_claim_nested_two_modules_deep_is_refused_by_name_not_guessed_at() {
        let dir = tempfile::tempdir().unwrap();
        write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub mod outer {
    pub mod inner {
        pub struct Deep;
        impl Deep {
            pub fn new() -> Self { Deep }
            pub fn value(&self) -> u32 { 0 }
        }
    }
}
"#,
            )],
        );
        let err = discover_method_with_receiver(
            dir.path(),
            "outer::inner::Deep::value",
            &RouteTable::new(),
        )
        .unwrap_err();
        assert!(matches!(err, ReceiverError::UnsupportedModulePath));
    }

    // -- struct/enum parameters (this task, 2026-08-27) --------------------
    // `resolve_user_type` is the parameter counterpart of receiver
    // construction: the same constructor-call mechanism (rule 1), plus
    // direct field/variant construction when nothing is private (rule 2),
    // otherwise refused by name (rule 3). See the module doc above
    // `MAX_USER_TYPE_DEPTH` for the full section.

    fn discover_fn_in(dir: &Path, files: &[(&str, &str)], fn_name: &str) -> ContractFn {
        let lib = write_crate(dir, files);
        let mut cf = discover_fn(&lib, fn_name).unwrap();
        // `discover_fn` alone never enriches a struct/enum parameter --
        // that is `verify`'s own job (`enrich_contract_fn_user_types`,
        // called once `cf` is resolved), same as it is for a real run.
        enrich_contract_fn_user_types(&mut cf, dir, &RouteTable::new());
        cf
    }

    /// Rule 1: a private-field struct with a usable constructor becomes a
    /// `UserTypeCtor` parameter, and the fn earns a real verdict path
    /// (`is_fuzz_supported`).
    #[test]
    fn a_parameter_of_a_private_field_type_with_a_constructor_resolves_via_rule_1() {
        let dir = tempfile::tempdir().unwrap();
        let cf = discover_fn_in(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct TicketPool { capacity: u32 }
impl TicketPool {
    pub fn new(capacity: u32) -> Self { TicketPool { capacity } }
    pub fn capacity(&self) -> u32 { self.capacity }
}
#[ply::ensures(|result| *result % 2 == 0)]
pub fn doubled(p: TicketPool) -> u64 { p.capacity() as u64 * 2 }
"#,
            )],
            "doubled",
        );
        let RustType::UserTypeCtor(plan) = &cf.params[0].ty else {
            panic!("expected UserTypeCtor, got {:?}", cf.params[0].ty);
        };
        assert_eq!(plan.type_name, "TicketPool");
        assert_eq!(plan.constructor, "TicketPool::new");
        assert!(
            plan.operations.is_empty(),
            "a parameter's build carries no operation pool"
        );
        assert_eq!(plan.max_sequence_len, 0);
        assert!(cf.is_fuzz_supported());
        assert!(
            !cf.is_bounded_supported(),
            "struct/enum parameters are fuzz-tier only, matching the receiver mechanism they reuse"
        );
    }

    /// Rule 2: an all-public-fields struct becomes a `UserTypeFields`
    /// parameter.
    #[test]
    fn a_parameter_of_an_all_public_fields_struct_resolves_via_rule_2() {
        let dir = tempfile::tempdir().unwrap();
        let cf = discover_fn_in(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Point { pub x: i32, pub y: i32 }
#[ply::ensures(|result| *result >= 0)]
pub fn norm(p: Point) -> i64 { (p.x as i64).abs() + (p.y as i64).abs() }
"#,
            )],
            "norm",
        );
        let RustType::UserTypeFields(plan) = &cf.params[0].ty else {
            panic!("expected UserTypeFields, got {:?}", cf.params[0].ty);
        };
        assert_eq!(plan.type_name, "Point");
        let UserTypeShape::Struct(fields) = &plan.shape else {
            panic!("expected a struct shape");
        };
        assert_eq!(fields.len(), 2);
        assert!(cf.is_fuzz_supported());
    }

    /// Rule 2 for an enum: every variant's own fields, named, all
    /// resolved.
    #[test]
    fn a_parameter_of_an_all_public_enum_resolves_via_rule_2() {
        let dir = tempfile::tempdir().unwrap();
        let cf = discover_fn_in(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub enum Shape { Circle { radius: u32 }, Square { side: u32 }, Origin }
#[ply::ensures(|result| *result >= 0)]
pub fn area(s: Shape) -> i64 {
    match s {
        Shape::Circle { radius } => radius as i64,
        Shape::Square { side } => side as i64,
        Shape::Origin => 0,
    }
}
"#,
            )],
            "area",
        );
        let RustType::UserTypeFields(plan) = &cf.params[0].ty else {
            panic!("expected UserTypeFields, got {:?}", cf.params[0].ty);
        };
        let UserTypeShape::Enum(variants) = &plan.shape else {
            panic!("expected an enum shape");
        };
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].0, "Circle");
        assert_eq!(variants[2].1.len(), 0, "a unit variant has no fields");
    }

    /// Rule 3: no usable constructor and a private field -- refused by
    /// name, naming the type and the reason.
    #[test]
    fn a_parameter_with_no_constructor_and_a_private_field_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Locked { secret: u32 }
impl Locked {
    pub fn secret(&self) -> u32 { self.secret }
}
#[ply::ensures(|result| *result >= 0)]
pub fn read(l: Locked) -> u32 { l.secret() }
"#,
            )],
        );
        let mut cf = discover_fn(&src, "read").unwrap();
        assert!(
            matches!(cf.params[0].ty, RustType::Unsupported(_)),
            "unresolved until enrichment runs"
        );
        let crate_dir = dir.path();
        let refused = enrich_contract_fn_user_types(&mut cf, crate_dir, &RouteTable::new());
        assert_eq!(refused.len(), 1);
        let (param_name, type_name, reason) = &refused[0];
        assert_eq!(param_name, "l");
        assert_eq!(type_name, "Locked");
        assert!(
            reason.contains("Locked") && reason.contains("private"),
            "must name the type and say why: {reason}"
        );
        assert!(
            matches!(cf.params[0].ty, RustType::Unsupported(_)),
            "a refused type is left exactly as it was, not silently guessed at"
        );
        // §5.4b's generator hook, this task: a type with no other way in and
        // no route declared for it gains one more sentence naming the
        // declaration that would unlock it, rather than leaving a reader to
        // rediscover the feature on their own.
        assert!(
            reason.contains("routes:") && reason.contains("Locked"),
            "the refusal must say a declared route would unlock it: {reason}"
        );
    }

    /// §5.4b's generator hook (this task, 2026-09-02): the routeprobe's own
    /// case in miniature -- a private-field struct made only by a *free*
    /// function, which rule 1's constructor scan cannot see at all (it only
    /// ever looks inside `impl` blocks). Declaring a route in `ply.yaml`
    /// resolves it the same way an associated constructor already does.
    #[test]
    fn a_declared_route_to_a_free_function_builds_a_type_no_constructor_scan_can_reach() {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = RouteTable::new();
        routes.insert("Handle".to_string(), "open_handle".to_string());
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Handle { id: u32 }
pub fn open_handle(id: u32) -> Handle { Handle { id } }
#[ply::ensures(|result| *result >= 0)]
pub fn use_handle(h: &Handle) -> i64 { h.id as i64 }
"#,
            )],
        );
        let mut cf = discover_fn(&lib, "use_handle").unwrap();
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert!(
            refused.is_empty(),
            "a declared route must not be reported as a refusal: {refused:?}"
        );
        let RustType::UserTypeCtor(plan) = &cf.params[0].ty else {
            panic!(
                "expected UserTypeCtor via the declared route, got {:?}",
                cf.params[0].ty
            );
        };
        assert_eq!(plan.type_name, "Handle");
        assert_eq!(plan.constructor, "open_handle");
        assert_eq!(plan.ctor_params.len(), 1);
        assert_eq!(plan.ctor_params[0].name, "id");
        let route = plan.route.as_ref().expect("must carry a route mark");
        assert_eq!(route.declared_as, "open_handle");
        assert!(cf.is_fuzz_supported());
        assert!(
            !cf.is_bounded_supported(),
            "route-built parameters are fuzz-tier only, matching every other struct/enum \
             parameter"
        );
    }

    /// The same route, nested inside a `Vec` -- the composition grammar
    /// (2026-09-02) must recurse into a route-built element exactly as it
    /// already does for a constructor-built one, since a route-built value
    /// is still `RustType::UserTypeCtor` under the hood.
    #[test]
    fn a_declared_route_composes_inside_a_vec() {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = RouteTable::new();
        routes.insert("Handle".to_string(), "open_handle".to_string());
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Handle { id: u32 }
pub fn open_handle(id: u32) -> Handle { Handle { id } }
#[ply::ensures(|result| *result >= 0)]
pub fn use_many_handles(hs: Vec<Handle>) -> i64 { hs.len() as i64 }
"#,
            )],
        );
        let mut cf = discover_fn(&lib, "use_many_handles").unwrap();
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert!(refused.is_empty(), "{refused:?}");
        let RustType::Vec(inner) = &cf.params[0].ty else {
            panic!("expected Vec, got {:?}", cf.params[0].ty);
        };
        assert!(
            matches!(inner.as_ref(), RustType::UserTypeCtor(_)),
            "the element must resolve through the declared route too, got {inner:?}"
        );
        assert!(cf.is_fuzz_supported());
        assert!(
            cf.params[0].ty.uses_any_route(),
            "a route-built element nested inside a Vec must still be reported as route-built"
        );
    }

    /// `uses_any_route` must say `false` for an ordinary constructor-built
    /// value -- the mark exists to tell a route-built run apart from an
    /// ordinary one, and a type with no route declared for it is ordinary.
    #[test]
    fn uses_any_route_is_false_for_a_constructor_built_type() {
        let dir = tempfile::tempdir().unwrap();
        let cf = discover_fn_in(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct TicketPool { capacity: u32 }
impl TicketPool {
    pub fn new(capacity: u32) -> Self { TicketPool { capacity } }
}
#[ply::ensures(|result| *result % 2 == 0)]
pub fn doubled(p: TicketPool) -> u64 { p.capacity as u64 * 2 }
"#,
            )],
            "doubled",
        );
        assert!(!cf.params[0].ty.uses_any_route());
    }

    /// The stale-route requirement: renaming (or removing) the function a
    /// route names must fail loudly and name it -- never a silent fall
    /// through to rule 2's direct field construction (which would also
    /// refuse `Handle` here, for a different reason, and hide the real
    /// defect: the declaration itself no longer points at anything).
    #[test]
    fn a_stale_route_is_refused_loudly_naming_the_function() {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = RouteTable::new();
        routes.insert("Handle".to_string(), "open_handle".to_string());
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Handle { id: u32 }
pub fn renamed_open_handle(id: u32) -> Handle { Handle { id } }
#[ply::ensures(|result| *result >= 0)]
pub fn use_handle(h: &Handle) -> i64 { h.id as i64 }
"#,
            )],
        );
        let mut cf = discover_fn(&lib, "use_handle").unwrap();
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert_eq!(refused.len(), 1);
        let (_, type_name, reason) = &refused[0];
        assert_eq!(type_name, "Handle");
        assert!(
            reason.contains("open_handle"),
            "the refusal must name the stale function: {reason}"
        );
        assert!(
            reason.contains("could not find"),
            "the refusal must say Ply looked and failed, not merely that the type is \
             unsupported: {reason}"
        );
        assert!(
            matches!(cf.params[0].ty, RustType::Unsupported(_)),
            "a stale route must not be silently treated as though nothing were declared"
        );
    }

    /// Defect 1 (2026-09-02): a route naming a real, public function that
    /// Ply simply cannot see -- because it lives outside this crate, and
    /// route lookup only ever reads this crate's own source -- used to fail
    /// in total silence, reading exactly as if no route had been declared
    /// at all (the bug this task exists to fix). Fixed here: routes are now
    /// tried before the "is this type even declared in this crate" gate
    /// that used to bar the door, so a real function Ply cannot read the
    /// source of is refused by name, the same way a stale local one already
    /// was.
    #[test]
    fn a_route_to_a_type_outside_the_crate_with_no_input_types_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = RouteTable::new();
        routes.insert(
            "OsString".to_string(),
            "std::ffi::OsString::from".to_string(),
        );
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
#[ply::ensures(|result| *result >= 0)]
pub fn use_os_string(o: std::ffi::OsString) -> i64 { 0 }
"#,
            )],
        );
        let mut cf = discover_fn(&lib, "use_os_string").unwrap();
        assert!(
            matches!(cf.params[0].ty, RustType::Unsupported(_)),
            "unresolved until enrichment runs"
        );
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert_eq!(
            refused.len(),
            1,
            "a declared route Ply cannot use must be refused by name, never silently dropped: \
             {refused:?}"
        );
        let (param_name, type_name, reason) = &refused[0];
        assert_eq!(param_name, "o");
        assert_eq!(type_name, "OsString");
        assert!(
            reason.contains("routes:")
                && reason.contains("OsString")
                && reason.contains("std::ffi::OsString::from"),
            "the refusal must name the declared route, not read as though nothing had been \
             declared: {reason}"
        );
        assert!(
            matches!(cf.params[0].ty, RustType::Unsupported(_)),
            "a route Ply could not use must not be silently guessed at"
        );
    }

    /// The same defect, proven for a route naming a path that is not stale
    /// (it never existed) rather than renamed -- distinct evidence from
    /// `a_stale_route_is_refused_loudly_and_names_the_function`'s rename
    /// case, both required by this task's own brief.
    #[test]
    fn a_route_naming_a_path_that_does_not_exist_anywhere_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = RouteTable::new();
        routes.insert("Handle".to_string(), "totally::bogus::path".to_string());
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Handle { id: u32 }
#[ply::ensures(|result| *result >= 0)]
pub fn use_handle(h: &Handle) -> i64 { h.id as i64 }
"#,
            )],
        );
        let mut cf = discover_fn(&lib, "use_handle").unwrap();
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert_eq!(refused.len(), 1, "{refused:?}");
        let (_, type_name, reason) = &refused[0];
        assert_eq!(type_name, "Handle");
        assert!(
            reason.contains("routes:") && reason.contains("totally::bogus::path"),
            "the refusal must name the declared route: {reason}"
        );
        assert!(
            reason.contains("could not find"),
            "must say Ply looked and failed: {reason}"
        );
    }

    /// Defect 3, found while verifying this task (coordinator probe): a
    /// declared route was silently never called whenever the ordinary
    /// constructor scan (rule 1) *also* happened to find a way to build the
    /// same type -- `StatusSet::new()` is both a legitimate rule-1
    /// constructor (a zero-argument, `Self`-returning associated fn) and
    /// the exact function a route names, and rule 1 ran first and won
    /// without a trace. An explicit `routes:` declaration is the author
    /// stating which function to call; it must never be silently shadowed
    /// by a rule that happens to find something else first. Fixed by
    /// checking a declared route before rule 1 runs at all, so a route
    /// always wins when one is declared, working or not.
    #[test]
    fn a_declared_route_wins_over_an_ordinary_constructor_rule_finds_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = RouteTable::new();
        routes.insert("StatusSet".to_string(), "StatusSet::other".to_string());
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
#[derive(Debug, Clone, Copy)]
pub struct StatusSet(u8);
impl StatusSet {
    pub const fn new() -> Self { StatusSet(0) }
    pub const fn other() -> Self { StatusSet(1) }
    pub fn len(&self) -> usize { self.0.count_ones() as usize }
}
#[ply::ensures(|result| *result <= 8)]
pub fn set_len(s: &StatusSet) -> usize { s.len() }
"#,
            )],
        );
        let mut cf = discover_fn(&lib, "set_len").unwrap();
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert!(refused.is_empty(), "{refused:?}");
        let RustType::UserTypeCtor(plan) = &cf.params[0].ty else {
            panic!("expected UserTypeCtor, got {:?}", cf.params[0].ty);
        };
        assert_eq!(
            plan.constructor, "StatusSet::other",
            "the declared route must be used even though rule 1's own constructor scan would \
             have found `StatusSet::new` first: got {}",
            plan.constructor
        );
        assert!(
            plan.route.is_some(),
            "a route-built value must carry its route mark"
        );
    }

    /// Defect 2 (2026-09-02): a route to a function outside the crate
    /// declares its own input types, since Ply has no source to read them
    /// from -- `routes: { OsString: std::ffi::OsString::from(String) }`.
    /// Ply builds the declared `String` the ordinary way and calls the
    /// named path directly, never reading `OsString::from`'s real
    /// signature at all.
    #[test]
    fn a_declared_route_with_explicit_input_types_builds_a_cross_crate_type() {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = RouteTable::new();
        routes.insert(
            "OsString".to_string(),
            "std::ffi::OsString::from(String)".to_string(),
        );
        let lib = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
#[ply::ensures(|result| *result >= 0)]
pub fn use_os_string(o: std::ffi::OsString) -> i64 { o.len() as i64 }
"#,
            )],
        );
        let mut cf = discover_fn(&lib, "use_os_string").unwrap();
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert!(
            refused.is_empty(),
            "a well-formed cross-crate route must not be reported as broken: {refused:?}"
        );
        let RustType::UserTypeCtor(plan) = &cf.params[0].ty else {
            panic!("expected UserTypeCtor, got {:?}", cf.params[0].ty);
        };
        assert_eq!(plan.type_name, "OsString");
        assert_eq!(plan.constructor, "std::ffi::OsString::from");
        assert_eq!(plan.import_path, "std::ffi::OsString");
        assert_eq!(plan.ctor_params.len(), 1);
        assert!(matches!(plan.ctor_params[0].ty, RustType::String));
        let route = plan.route.as_ref().expect("must carry a route mark");
        assert_eq!(route.declared_as, "std::ffi::OsString::from");
        assert!(
            route.outside_crate,
            "a cross-crate route must be marked as such, so codegen never assumes its type \
             lives in the target crate"
        );
        assert!(cf.is_fuzz_supported());
    }

    /// Defect 1's third required scenario: a route naming a type nothing in
    /// the document ever asks Ply to build. Nothing in `enrich_contract_fn_
    /// user_types`'s own per-parameter walk would ever visit it -- so a
    /// broken route of this shape needs its own check, run once per
    /// declared route regardless of whether anything uses it, or "every
    /// declared route is used or refused by name" would be false exactly
    /// when nothing asks.
    #[test]
    fn a_route_naming_a_type_nothing_in_the_document_uses_is_still_validated() {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = RouteTable::new();
        routes.insert("NeverAsked".to_string(), "totally::bogus::path".to_string());
        write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
#[ply::ensures(|result| *result >= 0)]
pub fn ordinary(n: u32) -> i64 { n as i64 }
"#,
            )],
        );
        let reason = validate_declared_route(dir.path(), &routes, "NeverAsked")
            .expect("a broken route must be reported even when nothing uses it");
        assert!(
            reason.contains("NeverAsked") && reason.contains("totally::bogus::path"),
            "the refusal must name the route: {reason}"
        );
    }

    /// The never-impossible-value proof, at the unit level: a type whose
    /// invariant is maintained by its constructor (both fields private)
    /// resolves *only* through the constructor -- rule 2 never even gets a
    /// chance, because the fields are not public.
    #[test]
    fn a_constructor_maintained_invariant_type_never_resolves_via_direct_fields() {
        let dir = tempfile::tempdir().unwrap();
        let cf = discover_fn_in(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Bucket { capacity: u32, tokens: u32 }
impl Bucket {
    pub fn new(capacity: u32) -> Self { Bucket { capacity, tokens: capacity } }
}
#[ply::ensures(|result| *result)]
pub fn ok(b: Bucket) -> bool { b.tokens <= b.capacity }
"#,
            )],
            "ok",
        );
        match &cf.params[0].ty {
            RustType::UserTypeCtor(plan) => assert_eq!(plan.constructor, "Bucket::new"),
            other => {
                panic!("expected UserTypeCtor (the only route private fields allow), got {other:?}")
            }
        }
    }

    /// A bare name that is not a struct/enum this crate declares at all
    /// (a generic, or simply unknown) is left `Unsupported` silently --
    /// `NotFound` is not itself reported, since it was never a candidate.
    #[test]
    fn a_type_name_this_crate_does_not_declare_is_left_unsupported_silently() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_crate(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Whatever;
#[ply::ensures(|result| *result >= 0)]
pub fn f(x: NotDeclaredAnywhere) -> u32 { 0 }
"#,
            )],
        );
        let mut cf = discover_fn(&src, "f").unwrap();
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &RouteTable::new());
        assert!(
            refused.is_empty(),
            "a name that is not a struct/enum this crate declares must not be reported as a \
             refused struct/enum -- it was never a candidate: {refused:?}"
        );
        assert!(matches!(cf.params[0].ty, RustType::Unsupported(_)));
    }

    /// A bare name declared in more than one file is refused as ambiguous,
    /// never guessed at.
    #[test]
    fn a_bare_name_declared_twice_is_ambiguous_not_guessed_at() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_crate(
            dir.path(),
            &[
                (
                    "lib.rs",
                    r#"
mod a;
mod b;
#[ply::ensures(|result| *result >= 0)]
pub fn f(x: Dup) -> u32 { 0 }
"#,
                ),
                ("a.rs", r#"pub struct Dup { pub n: u32 }"#),
                ("b.rs", r#"pub struct Dup { pub n: u32 }"#),
            ],
        );
        let mut cf = discover_fn(&src, "f").unwrap();
        let refused = enrich_contract_fn_user_types(&mut cf, dir.path(), &RouteTable::new());
        assert_eq!(refused.len(), 1);
        assert!(
            refused[0].2.contains("more than one"),
            "must name the ambiguity: {:?}",
            refused[0]
        );
    }

    /// Recursion: a constructor argument that is itself another buildable
    /// user type resolves too (`Quota::new`'s own `RefillRate` argument, in
    /// the rate-limiter fixture, is exactly this shape).
    #[test]
    fn a_constructor_argument_that_is_itself_a_user_type_resolves_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let cf = discover_fn_in(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct RefillRate { tokens: u32 }
impl RefillRate {
    pub fn per_second(tokens: u32) -> Self { RefillRate { tokens } }
}
pub struct Quota { capacity: u32, refill: RefillRate }
impl Quota {
    pub fn new(capacity: u32, refill: RefillRate) -> Self { Quota { capacity, refill } }
}
#[ply::ensures(|result| *result >= 0)]
pub fn f(q: Quota) -> u32 { q.capacity }
"#,
            )],
            "f",
        );
        let RustType::UserTypeCtor(plan) = &cf.params[0].ty else {
            panic!("expected UserTypeCtor for Quota, got {:?}", cf.params[0].ty);
        };
        assert_eq!(plan.type_name, "Quota");
        assert_eq!(plan.ctor_params.len(), 2);
        assert!(
            matches!(plan.ctor_params[1].ty, RustType::UserTypeCtor(_)),
            "`refill`'s own type must have resolved recursively, not stayed `Unsupported`: {:?}",
            plan.ctor_params[1].ty
        );
    }

    // -- composition (TODO.md, "make the sampling engine's decision
    // recursive, and add slices", 2026-09-02): every shape the sampling
    // engine already builds alone must also compose inside `Option`,
    // `Result`, a list, a set, a map, a fixed array, a slice, a tuple, and
    // `Box` -- the proof engine's own supported-parameter list is a
    // standing obligation that must NOT move a single inch while this
    // happens. This block is written, and watched to fail, before the
    // grammar work that makes the five `is_fuzz_supported` assertions below
    // pass -- see the task's own commit history for the red run.

    /// The regression pin the task requires **before** any grammar change:
    /// every shape the *bounded* (Kani) engine accepts today stays accepted,
    /// and none of the four newly-added composition shapes -- nor a
    /// previously-unsupported inner type newly allowed to nest inside an
    /// old composite -- ever becomes bounded-supported. If this test ever
    /// goes red, Ply has started claiming exhaustive proof over a shape
    /// nobody has measured, which is worse than the defect this task fixes.
    #[test]
    fn the_bounded_proof_engines_own_supported_list_never_widens() {
        // Unchanged shapes: every one of these was bounded-supported before
        // this task and must still be, byte for byte.
        for ty in [
            RustType::U32,
            RustType::Bool,
            RustType::Char,
            RustType::Usize,
            RustType::VecU8,
            RustType::Duration,
            RustType::NonZero(Box::new(RustType::U32)),
            RustType::Option(Box::new(RustType::U32)),
            RustType::Result(Box::new(RustType::U32), Box::new(RustType::U8)),
            RustType::Array(Box::new(RustType::U32), 4),
        ] {
            assert!(
                ty.is_bounded_supported(),
                "{ty:?} was bounded-supported before this task and must still be"
            );
        }
        // Unchanged shapes that were already refused on the proof engine
        // and must stay refused.
        for ty in [
            RustType::F32,
            RustType::String,
            RustType::Vec(Box::new(RustType::U32)),
            RustType::BTreeSet(Box::new(RustType::U32)),
        ] {
            assert!(
                !ty.is_bounded_supported(),
                "{ty:?} was already refused on the proof engine and must stay refused"
            );
        }
        // The four brand-new composition shapes: never bounded-supported,
        // whatever they wrap.
        for ty in [
            RustType::Slice(Box::new(RustType::U8)),
            RustType::Slice(Box::new(RustType::String)),
            RustType::Tuple(vec![RustType::U32, RustType::String]),
            RustType::BTreeMap(Box::new(RustType::U32), Box::new(RustType::String)),
            RustType::BoxT(Box::new(RustType::U32)),
        ] {
            assert!(
                !ty.is_bounded_supported(),
                "{ty:?} is a brand-new composition shape and must never be bounded-supported"
            );
        }
        // A previously-refused inner type (`String`, a user struct) newly
        // allowed to *nest* for the sampling engine must still be refused
        // on the proof engine when nested -- composition only ever widens
        // the fuzz side.
        for ty in [
            RustType::Option(Box::new(RustType::String)),
            RustType::Vec(Box::new(RustType::String)),
            RustType::Array(Box::new(RustType::String), 3),
        ] {
            assert!(
                !ty.is_bounded_supported(),
                "{ty:?} nests a shape the proof engine has never built and must stay refused there"
            );
        }
    }

    #[test]
    fn an_optional_string_is_fuzz_supported_via_composition() {
        let ty = rust_type_from_source("Option<String>").unwrap();
        assert_eq!(ty, RustType::Option(Box::new(RustType::String)));
        assert!(
            ty.is_fuzz_supported(),
            "a `String` is buildable alone -- nesting it inside `Option` must not refuse it: {ty:?}"
        );
        assert!(
            !ty.is_bounded_supported(),
            "the proof engine's own list does not change: {ty:?}"
        );
    }

    #[test]
    fn a_list_of_strings_is_fuzz_supported_via_composition() {
        let ty = rust_type_from_source("Vec<String>").unwrap();
        assert_eq!(ty, RustType::Vec(Box::new(RustType::String)));
        assert!(
            ty.is_fuzz_supported(),
            "a `Vec` of anything buildable must compose, not just a scalar element: {ty:?}"
        );
        assert!(!ty.is_bounded_supported());
    }

    #[test]
    fn a_shared_reference_to_a_slice_is_fuzz_supported() {
        // Not handled as a shape at all before this task -- `&[T]` fell
        // straight through to `Unsupported` (V0505).
        let dir = tempfile::tempdir().unwrap();
        let path = write_src(
            dir.path(),
            r#"
#[ply::ensures(|result| *result >= 0)]
pub fn total(xs: &[u32]) -> u64 { xs.iter().map(|x| *x as u64).sum() }
"#,
        );
        let cf = discover_fn(&path, "total").unwrap();
        assert!(
            cf.params[0].by_ref,
            "a slice parameter is only ever reachable through a reference"
        );
        assert_eq!(cf.params[0].ty, RustType::Slice(Box::new(RustType::U32)));
        assert!(
            cf.is_fuzz_supported(),
            "&[u32] must be sample-buildable -- build the owned Vec<u32> and lend it, the same \
             trick already used for &Vec<u8>: {:?}",
            cf.params[0].ty
        );
        assert!(
            !cf.is_bounded_supported(),
            "a slice earns no more than Vec<T> already does on the proof engine: never bounded"
        );
    }

    #[test]
    fn a_list_of_a_user_struct_is_fuzz_supported_via_composition() {
        let dir = tempfile::tempdir().unwrap();
        let cf = discover_fn_in(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Doc { n: u32 }
impl Doc { pub fn new(n: u32) -> Self { Doc { n } } }
#[ply::ensures(|result| *result >= 0)]
pub fn many(a: Vec<Doc>) -> i64 { a.len() as i64 }
"#,
            )],
            "many",
        );
        assert!(
            matches!(&cf.params[0].ty, RustType::Vec(inner) if matches!(inner.as_ref(), RustType::UserTypeCtor(_))),
            "a `Vec` of a user struct Ply already knows how to build must resolve recursively, \
             not stay Unsupported: {:?}",
            cf.params[0].ty
        );
        assert!(
            cf.is_fuzz_supported(),
            "every element is buildable alone (`Doc::new`) -- the list around it must not \
             refuse the whole parameter: {:?}",
            cf.params[0].ty
        );
        assert!(!cf.is_bounded_supported());
    }

    #[test]
    fn an_optional_user_struct_is_fuzz_supported_via_composition() {
        let dir = tempfile::tempdir().unwrap();
        let cf = discover_fn_in(
            dir.path(),
            &[(
                "lib.rs",
                r#"
pub struct Doc { n: u32 }
impl Doc { pub fn new(n: u32) -> Self { Doc { n } } }
#[ply::ensures(|result| *result >= 0)]
pub fn optional(a: Option<Doc>) -> i64 { a.map(|d| d.n).unwrap_or(0) as i64 }
"#,
            )],
            "optional",
        );
        assert!(
            matches!(&cf.params[0].ty, RustType::Option(inner) if matches!(inner.as_ref(), RustType::UserTypeCtor(_))),
            "an `Option` of a user struct Ply already knows how to build must resolve \
             recursively, not stay Unsupported: {:?}",
            cf.params[0].ty
        );
        assert!(cf.is_fuzz_supported(), "{:?}", cf.params[0].ty);
        assert!(!cf.is_bounded_supported());
    }
}
