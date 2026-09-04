//! M4 fuzz-check codegen (§5.4c `fuzz(n)`, §10 M4): builds one proptest-based
//! `#[test]` per contracted fn (ints biased small, `Vec`/`BTreeSet` length
//! 0-8, `requires` as a rejection filter), plus the `test` check's generated
//! artifacts (`examples` entries and auto-generated boundary "direct contract
//! cases"). All three land as `#[cfg(test)] mod {fn}_harness { ... }` in the
//! generated harness crate under `target/ply/fuzz/` (§5.4c), which
//! `engines::fuzz`/`engines::mutants` then run.
//!
//! Deliberately out of scope this session (see docs/m4-findings.md): struct
//! parameters ("field-by-field" fuzzing) -- `harness::RustType` has no struct
//! variant yet, so nothing here needs to handle one.
//!
//! On a shrunk fuzz failure, the concrete values are printed to stdout in a
//! small tagged marker line (`PLY_FUZZED_CEX|...`) that `engines::fuzz`
//! parses back into the *same* `WitnessValue` type Kani witnesses decode
//! into -- so the shrunk failure renders through the *same*
//! `contract_rt::render_cex_test` the Kani path uses (the D7 plan's "two
//! consumers, one renderer", now both wired). Any container `WitnessValue`
//! cannot spell -- every `BTreeSet`, and a `Vec` of anything but `u8` -- has
//! no literal form this renderer can write, so the caller reports that case
//! as a witness-only violation (`W0541`), never a fabricated input.

use std::collections::BTreeSet;

use anyhow::{Result, bail};
use quote::ToTokens;
use syn::Expr;
use syn::visit::Visit;

use crate::harness;
use crate::harness::{ContractFn, Param, RustType};
use crate::model::Check;

/// The seed a `fuzz(n)` run uses, derived from the function it checks
/// (2026-08-25). Until now the generated harness built its runner with
/// `Config { cases, ..default() }`, whose RNG is seeded from entropy and
/// recorded nowhere: vetting 004's finding 4 measured six runs of identical
/// source splitting 3-3 between a clean pass and the real panic, with the
/// run that found the bug unreplayable and the run that missed it
/// indistinguishable from a real pass.
///
/// Deriving it from the fn's name and contract text rather than from a
/// constant means two functions in one run do not sample the same draw
/// sequence, and a changed contract gets a fresh one -- while identical
/// source always replays identically, which is the property the fix is for.
/// `--seed` overrides it for a run.
///
/// **This buys replay and auditability, not detection power.** A seeded
/// coin flip is still a coin flip: 256 uniform samples that miss an overflow
/// beginning at ~29% of the input range still miss it, every time, now
/// reproducibly. The reliability story for the fuzz tier is seed plus
/// `mutate`'s kill signal, never the seed alone.
pub fn derive_seed(fn_name: &str, contract_text: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        // FNV-1a over the same input with a different per-chunk salt: a
        // stable, dependency-free hash (nothing here needs to resist an
        // adversary, only entropy).
        let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for b in fn_name
            .as_bytes()
            .iter()
            .chain(b"\x1f")
            .chain(contract_text.as_bytes())
        {
            h ^= *b as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
        chunk.copy_from_slice(&h.to_le_bytes());
    }
    out
}

/// The seed as it appears in the §8 envelope and in `--seed`.
pub fn seed_hex(seed: &[u8; 32]) -> String {
    seed.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses a `--seed` value back into 32 bytes. Returns `None` for anything
/// that is not exactly 64 hex characters, so a mistyped seed is refused
/// rather than silently padded into a different run.
pub fn seed_from_hex(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// The length bound (in **characters**, never bytes) Ply's generated
/// `String` strategy samples up to -- named once so `strategy_expr` and
/// `boundary_literals` agree on the same number, and so a reader (or a
/// future change) sees one place to look rather than a constant copied
/// twice. Chosen deliberately, not measured (task brief, 2026-08-27, unlike
/// `Duration`'s benchmarked bound): long enough that a typical fixed-size
/// "preview"/"truncate to N" idiom (commonly 8-20 characters in real code)
/// has room to be exercised past its own boundary, short enough that
/// proptest's shrinker stays fast. Mirrors `Vec`/`BTreeSet`'s own bound
/// (0..=8 *elements*) in spirit -- a small, disclosed cap, never unbounded
/// -- but a string genuinely needs more raw length than a numeric vector to
/// reach the same kind of boundary bug, hence a larger number here.
///
/// Character count, not byte count, is what is bounded: a 32-character
/// string of 4-byte emoji is a real (if unusual) input up to 128 bytes, and
/// bounding by *bytes* instead would silently make multi-byte content rarer
/// exactly where this type's own value proposition lives (see
/// `RustType::String`'s doc: "the richest bug territory").
pub(crate) const STRING_MAX_CHARS: u32 = 32;

/// One proptest strategy expression for `ty`, biased toward small magnitudes
/// (§10 M4: "ints biased small"). `Unsupported` must never reach here --
/// callers gate on `RustType::is_fuzz_supported` first.
fn strategy_expr(ty: &RustType) -> Result<String> {
    Ok(match ty {
        RustType::Bool => "proptest::prelude::any::<bool>()".to_string(),
        RustType::U8 | RustType::U16 | RustType::U32 | RustType::U64 | RustType::Usize => {
            let name = ty.scalar_rust_name().expect("scalar");
            format!(
                "proptest::prop_oneof![3 => 0{name}..=16{name}, 1 => proptest::prelude::any::<{name}>()]"
            )
        }
        RustType::I8 | RustType::I16 | RustType::I32 | RustType::I64 | RustType::Isize => {
            let name = ty.scalar_rust_name().expect("scalar");
            format!(
                "proptest::prop_oneof![3 => -16{name}..=16{name}, 1 => proptest::prelude::any::<{name}>()]"
            )
        }
        RustType::VecU8 => {
            "proptest::collection::vec(proptest::prelude::any::<u8>(), 0..=8)".to_string()
        }
        RustType::Vec(inner) => format!(
            "proptest::collection::vec({}, 0..=8)",
            strategy_expr(inner)?
        ),
        RustType::BTreeSet(inner) => {
            format!(
                "proptest::collection::btree_set({}, 0..=8)",
                strategy_expr(inner)?
            )
        }
        // `char` still has proptest's own blanket `Arbitrary` impl -- no
        // small-magnitude bias needed, the interesting values are the code
        // points, not a size.
        RustType::Char => "proptest::prelude::any::<char>()".to_string(),
        // `Option<T>`, `Result<T, E>` and `[T; N]` used to reuse proptest's
        // own blanket `Arbitrary` impl (`any::<Option<T>>()`) here, which
        // only ever worked because -- until this task -- the parser itself
        // refused to build one of these three around anything but a leaf
        // scalar or another already-composite-constructible shape (never a
        // `String`, a `Vec`, a user struct, ...). Composition (TODO.md,
        // 2026-09-02) widens what these three can wrap, and `any::<T>()`
        // would either not compile for the new inner shapes (no blanket
        // `Arbitrary` for a user struct) or -- worse, for `String` -- quietly
        // swap in proptest's own unbounded, uncurated `Arbitrary` sampling
        // in place of this module's own deliberate content/length/NaN
        // decisions (`RustType::String`/`F32`'s own doc comments). So all
        // three are now built explicitly, recursing into this same function
        // for the inner strategy -- the exact same combinator `Vec`/
        // `BTreeSet` above already use, extended to every shape here.
        RustType::Option(inner) => {
            format!("proptest::option::of({})", strategy_expr(inner)?)
        }
        // Draws *both* an `Ok` value and an `Err` value on every case (a
        // small, disclosed inefficiency, never a correctness problem: only
        // one is kept) rather than naming either type explicitly -- neither
        // has a spellable name here in general (`rust_name()` only knows
        // the old leaf/composite-constructible vocabulary, and this arm now
        // reaches `String`, a user struct, or another container). The
        // `if`/`else` gives the compiler both branches together, which is
        // enough to infer the concrete `Result<OkT, ErrT>` from the two
        // already-concrete leaf values alone -- no type name spelled by
        // this codegen at all.
        RustType::Result(ok, err) => format!(
            "proptest::strategy::Strategy::prop_map(\
             (proptest::prelude::any::<bool>(), {ok_s}, {err_s}), \
             |(__ply_ok, __ply_ok_v, __ply_err_v)| if __ply_ok {{ Ok(__ply_ok_v) }} else {{ \
             Err(__ply_err_v) }})",
            ok_s = strategy_expr(ok)?,
            err_s = strategy_expr(err)?,
        ),
        // Built via a `Vec` of exactly `n` elements, then converted with
        // `TryInto` -- never `any::<[T; N]>()` (which needed `T: Arbitrary`,
        // wrong for the new inner shapes this arm now reaches) and never a
        // literal `[s1, s2, ..., sn]` array of strategies (whether proptest
        // implements `Strategy` for an array of strategies at every arity
        // this codegen could produce is not something this module verifies,
        // so it does not rely on it). `0..=8`'s own `Vec` combinator
        // narrowed to `n..=n` always yields exactly `n` elements, so the
        // `.try_into().unwrap()` never fails.
        RustType::Array(inner, n) => format!(
            "proptest::strategy::Strategy::prop_map(\
             proptest::collection::vec({elem}, {n}..={n}), \
             |__ply_v| <[_; {n}]>::try_from(__ply_v).unwrap())",
            elem = strategy_expr(inner)?,
        ),
        // A tuple of independent strategies -- proptest implements
        // `Strategy` for a plain Rust tuple of two or more strategies
        // directly (already relied on above, `Duration`'s own pair), which
        // is why 2+ elements are spliced as a bare tuple. Rust's own tuple
        // syntax needs its own two edge cases: `()` is a value, never a
        // `Strategy`, so an empty tuple type reuses `Just(())`; a one-tuple
        // needs the trailing comma (`(v,)`) that a *strategy* tuple of one
        // element does not parse as (`(s)` is just `s`, not a 1-tuple), so
        // it is built via `prop_map` instead of spliced as one.
        RustType::Tuple(items) => match items.len() {
            0 => "proptest::strategy::Just(())".to_string(),
            1 => format!(
                "proptest::strategy::Strategy::prop_map({}, |__ply_v| (__ply_v,))",
                strategy_expr(&items[0])?
            ),
            _ => {
                let elems: Vec<String> = items.iter().map(strategy_expr).collect::<Result<_>>()?;
                format!("({})", elems.join(", "))
            }
        },
        // A shared reference to `[T]` is built the same way `&Vec<u8>`
        // already is (see `RustType::VecU8`'s own doc): sample an owned
        // `Vec<T>` -- the call site (`by_ref`) lends it as `&name`, and
        // `Vec<T>: Deref<Target = [T]>` coerces that into the `&[T]` the
        // real function wants. No second construction mechanism.
        RustType::Slice(inner) => format!(
            "proptest::collection::vec({}, 0..=8)",
            strategy_expr(inner)?
        ),
        RustType::BTreeMap(key, value) => format!(
            "proptest::collection::btree_map({}, {}, 0..=8)",
            strategy_expr(key)?,
            strategy_expr(value)?
        ),
        // An owned wrapper: build `T` and box it. `Strategy::prop_map` is
        // exactly what every other wrapping shape in this file already uses.
        RustType::BoxT(inner) => format!(
            "proptest::strategy::Strategy::prop_map({}, Box::new)",
            strategy_expr(inner)?
        ),
        // Never `any::<NonZeroU32>()`: proptest's own coverage of the
        // `NonZero` family is not this module's concern one way or the
        // other -- the inner integer's own (small-biased) strategy is
        // reused and zero is folded up to one, so the value built is
        // always the type's own non-zero invariant, on this engine too.
        // `Strategy::prop_map` is called fully qualified (never `.prop_map`
        // as a method): the generated harness module never `use`s
        // `proptest::strategy::Strategy`, and method-call syntax did not
        // find the trait on the concrete strategy type `prop_oneof!`/a
        // tuple builds -- measured directly against this fixture (`.prop_map`
        // on a `TupleUnion` and on a raw tuple both failed with "no method
        // named `prop_map` found", E0599, before this fix).
        RustType::NonZero(inner) => {
            let inner_strategy = strategy_expr(inner)?;
            let inner_name = inner.scalar_rust_name().expect("nonzero inner is scalar");
            let suffix = inner.nonzero_suffix().expect("nonzero inner is valid");
            format!(
                "proptest::strategy::Strategy::prop_map({inner_strategy}, |v| \
                 std::num::NonZero{suffix}::new(if v == 0 {{ 1{inner_name} }} else {{ v }}).unwrap())"
            )
        }
        // A pair of independent strategies, never proptest's own `Arbitrary`
        // for `Duration` (this crate does not depend on that feature and
        // does not need to): whole seconds unconstrained, nanoseconds kept
        // under one billion so the value built is always one the standard
        // library could have returned. Same fully-qualified `Strategy::
        // prop_map` call as `NonZero` above, for the same reason.
        RustType::Duration => "proptest::strategy::Strategy::prop_map(\
             (proptest::prelude::any::<u64>(), 0u32..1_000_000_000u32), \
             |(s, n)| std::time::Duration::new(s, n))"
            .to_string(),
        // The NaN/infinity decision (task brief, 2026-08-27), made
        // deliberately rather than by accident: proptest's own
        // `prelude::any::<f64>()` samples the *entire* bit-pattern space,
        // NaN and both infinities included. A generated NaN makes almost
        // any postcondition comparison false (`NaN >= x` is false for
        // every `x`, `NaN == NaN` is false), so it would report a broken
        // promise on an input the real program may never produce -- a
        // false counterexample, which this project treats as nearly as
        // damaging as a false pass (both end with the tool switched off).
        // So the default excludes NaN and infinity: `POSITIVE | NEGATIVE`
        // (both signs) combined with `NORMAL | SUBNORMAL | ZERO` (every
        // *finite* class), deliberately never `INFINITE`, `QUIET_NAN`, or
        // `SIGNALING_NAN`. `W0518` (verify.rs) names this choice on every
        // run over a float-shaped fn, so it is visible rather than a silent
        // default a user could only discover by reading this file.
        //
        // Pinned by `strategy_expr_tests::float_strategy_excludes_nan_and_infinity_by_default`
        // below -- reversing this decision (back to a bare `any::<fN>()`)
        // fails that test rather than silently reintroducing false
        // counterexamples.
        RustType::F32 => "(proptest::num::f32::POSITIVE | proptest::num::f32::NEGATIVE \
             | proptest::num::f32::NORMAL | proptest::num::f32::SUBNORMAL \
             | proptest::num::f32::ZERO)"
            .to_string(),
        RustType::F64 => "(proptest::num::f64::POSITIVE | proptest::num::f64::NEGATIVE \
             | proptest::num::f64::NORMAL | proptest::num::f64::SUBNORMAL \
             | proptest::num::f64::ZERO)"
            .to_string(),
        // The content/length decision (task brief, 2026-08-27), made
        // deliberately rather than by accident, mirroring the float
        // NaN/infinity precedent immediately above:
        //
        // **Length**: 0..=`STRING_MAX_CHARS` *characters* -- bounded for
        // the same reason `Vec`/`BTreeSet` are (an unbounded string is not
        // sensible), disclosed and named in one place rather than assumed
        // (see that constant's own doc for the exact number and why).
        //
        // **Content**: a 9-to-1 mix of ordinary ASCII printable text
        // (`0x20..=0x7E`, biased heavily because it is the overwhelmingly
        // common case for real user-facing strings, the same "biased
        // small" philosophy the integer strategies above already use) and
        // genuine multi-byte Unicode (`0xA0..=0x10FFFF` -- accented
        // letters, CJK, symbols, emoji). Multi-byte content is deliberately
        // **included**, not excluded: unlike a float's NaN, any value a
        // `String` parameter can hold in real Rust code can already be
        // multi-byte UTF-8 (the type does not admit anything else), and
        // byte-vs-character confusion -- slicing or truncating by byte
        // count instead of char count -- is exactly the encoding/truncation
        // bug class this type exists to catch (task brief: "the richest
        // bug territory").
        //
        // **Excluded by default**: ASCII/Latin-1 *control* characters
        // (`0x00..=0x1F`, `0x7F..=0x9F`) -- the same "exclude the class
        // most likely to be a false alarm" reasoning as the float NaN
        // exclusion. A raw control byte (NUL, ESC, a C1 control) is the
        // input class ordinary user-facing text is least likely to
        // actually contain, and the class most likely to trip an unrelated
        // assumption (a terminal, a log line, a CSV cell) rather than the
        // checked function's own logic -- a false counterexample, which
        // this project treats as nearly as damaging as a false pass (both
        // end with the tool switched off). This run says nothing about
        // control-character handling, because it was never asked to --
        // exactly the float precedent's own words.
        //
        // Deliberately excludes `\u{0}`..`\u{1F}`/`\u{7F}`..`\u{9F}` from
        // *both* ranges below (`0x20` starts the first range, `0xA0` starts
        // the second, `0x7E` ends the first just before `0x7F`) -- pinned
        // by `strategy_expr_tests::string_strategy_excludes_control_
        // characters_by_default` below, the same way the float exclusion
        // is pinned: reversing this (widening either range to include a
        // control block) fails that test.
        RustType::String => format!(
            "proptest::strategy::Strategy::prop_map(\
             proptest::collection::vec(\
             proptest::prop_oneof![\
             9 => proptest::char::range('\\u{{20}}', '\\u{{7e}}'), \
             1 => proptest::char::range('\\u{{a0}}', '\\u{{10ffff}}')\
             ], 0..={STRING_MAX_CHARS}), \
             |__ply_cs: Vec<char>| __ply_cs.into_iter().collect::<String>())"
        ),
        RustType::Unsupported(t) => {
            bail!("cannot build a fuzz strategy for unsupported type `{t}`")
        }
        // Never reached: both are return-only shapes (`ContractFn::return_type`),
        // never a parameter's, so no caller ever asks this for one.
        RustType::SelfType | RustType::Unit => {
            bail!(
                "`{}` is a return-only shape and was never meant to reach a parameter strategy \
                 -- this is a Ply bug, not a user error",
                ty.display_name()
            )
        }
        // A *top-level* struct/enum parameter is never handed to
        // `strategy_expr` directly -- `plan_for_param`/`build_user_value_
        // stmt` draw a flat leaf-per-field strategy instead, spliced into
        // the checked call's own outer parameter tuple, so a top-level
        // struct/enum param's marker text can be precomputed field by field
        // before the call (`build_marker_stmt`) rather than needing the
        // built value itself to implement `Debug`.
        //
        // A *nested* struct/enum -- `Option<Doc>`, `Vec<Doc>`, a tuple
        // element, a `BTreeMap` key -- has no such outer splice to reach:
        // composition (TODO.md, 2026-09-02) recurses into this same
        // function from `Option`/`Result`/`Vec`/`BTreeSet`/`Array`/`Slice`/
        // `Tuple`/`BTreeMap`/`Box`'s own arms above, so it must have a real
        // self-contained `Strategy` of its own here -- `user_type_strategy_
        // expr` builds exactly that, reusing `build_user_value_stmt`'s same
        // three-rule construction, just closed over its own leaf tuple
        // instead of the caller's.
        // A struct/enum reached *through* composition (`Option<Doc>`,
        // `Vec<Doc>`, ...) never builds its own value here -- proptest's
        // own `prop_map`/`Map<S, F>` requires its *output* type to be
        // `Debug` (`Strategy` is only ever implemented for `Map<S, F>`
        // when `F::Output: Debug`), which this codegen cannot discharge
        // honestly for an arbitrary user struct (measured directly against
        // this exact shape: constructing `Doc` via `prop_map` inside
        // `Vec`'s own element strategy fails to compile with `E0277: Doc
        // doesn't implement Debug`, even though `Doc` is never itself
        // handed to `TestRunner::run`). So this only ever returns the
        // *raw leaf tuple* -- always scalars/`String`/other already-Debug
        // leaves, all the way down -- and the real value is constructed
        // afterwards from *ordinary* Rust code (`construct_from_raw_expr`,
        // called from `plan_for_param`), which carries no such bound at
        // all.
        RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => raw_user_type_strategy_expr(ty)?,
    })
}

/// The pattern (bare name, or a parenthesized tuple of names) and matching
/// strategy (bare strategy, or a parenthesized tuple of strategies) for a
/// list of same-length `names`/`strategies` -- the 0/1/N cases every
/// tuple-folding site in this file already handles for `cf.params` as a
/// whole, pulled out once so [`raw_user_type_strategy_expr`] does not carry
/// a fourth copy of them.
fn tuple_pattern_and_strategy(names: &[String], strategies: &[String]) -> (String, String) {
    match names.len() {
        0 => ("_".to_string(), "proptest::strategy::Just(())".to_string()),
        1 => (names[0].clone(), strategies[0].clone()),
        _ => (
            format!("({})", names.join(", ")),
            format!("({})", strategies.join(", ")),
        ),
    }
}

/// A self-contained proptest `Strategy` **expression** for a struct/enum
/// Ply builds *nested* inside another shape's own strategy (`Option<Doc>`,
/// `Vec<Doc>`, a tuple element, a `BTreeMap` key, ...) -- added 2026-09-02
/// for the composition task (TODO.md). Its `Value` is deliberately **not**
/// `ty`'s own constructed value: it is the *raw leaf tuple* every
/// constructor argument or field would need (recursing the same way for a
/// nested user type two levels deep), leaving the actual construction to
/// [`construct_from_raw_expr`], called once from [`plan_for_param`] on the
/// value this strategy hands back.
///
/// **Why not construct the value here, the way [`build_user_value_stmt`]
/// does for a top-level parameter's own preamble:** measured directly
/// against this exact shape (a `Vec<Doc>` parameter, `Doc` deriving no
/// `Debug` impl) -- `proptest::strategy::Strategy::prop_map`'s own trait
/// bound is `fn prop_map<O: fmt::Debug, F: Fn(Self::Value) -> O>(...)`
/// (`Map<S, F>` only implements `Strategy` at all when its *output* type is
/// `Debug`), so a `prop_map` that builds `Doc` fails to compile with
/// `error[E0277]: Doc doesn't implement Debug` regardless of anything
/// wrapped *around* it afterwards -- wrapping the final value in a
/// Debug-defeating newtype (tried first, reverted) does not help, because
/// the failure is in the *inner* `prop_map` that builds `Doc` in the first
/// place, before any outer wrapping ever runs. Returning only ever-`Debug`
/// leaves here, and constructing `Doc` afterwards with ordinary (non-
/// proptest) Rust code, carries no such bound at all.
fn raw_user_type_strategy_expr(ty: &RustType) -> Result<String> {
    match ty {
        RustType::UserTypeCtor(plan) => {
            let arg_strategies: Vec<String> = plan
                .ctor_params
                .iter()
                .map(|p| strategy_expr(&p.ty))
                .collect::<Result<_>>()?;
            let arg_names: Vec<String> = plan.ctor_params.iter().map(|p| p.name.clone()).collect();
            let (_, strategy) = tuple_pattern_and_strategy(&arg_names, &arg_strategies);
            Ok(strategy)
        }
        RustType::UserTypeFields(plan) => match &plan.shape {
            harness::UserTypeShape::Struct(fields) => {
                let field_strategies: Vec<String> = fields
                    .iter()
                    .map(|f| strategy_expr(&f.ty))
                    .collect::<Result<_>>()?;
                let field_names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
                let (_, strategy) = tuple_pattern_and_strategy(&field_names, &field_strategies);
                Ok(strategy)
            }
            // The raw shape of an enum is its own discriminant (which
            // variant) alongside *every* variant's own raw leaf tuple,
            // drawn unconditionally -- exactly mirroring
            // `build_user_value_stmt`'s own reasoning for a top-level enum
            // parameter (a few wasted draws per case, proptest does not
            // notice, and every variant's data is real regardless of which
            // one a case ends up using).
            harness::UserTypeShape::Enum(variants) => {
                let mut parts = vec![format!("0u8..{}u8", variants.len())];
                for (_, vfields) in variants {
                    let strategies: Vec<String> = vfields
                        .iter()
                        .map(|f| strategy_expr(&f.ty))
                        .collect::<Result<_>>()?;
                    let names: Vec<String> = vfields.iter().map(|f| f.name.clone()).collect();
                    let (_, strategy) = tuple_pattern_and_strategy(&names, &strategies);
                    parts.push(strategy);
                }
                Ok(format!("({})", parts.join(", ")))
            }
        },
        other => bail!(
            "raw_user_type_strategy_expr called on a non-user type `{}` -- this is a Ply bug, \
             not a user error",
            other.display_name()
        ),
    }
}

/// The exact reverse of [`raw_user_type_strategy_expr`]: given `var` bound
/// to the raw shape that function's strategy draws, builds an expression of
/// `ty`'s own real type, via ordinary (non-proptest) Rust code -- a
/// constructor call, or a field/variant literal, recursing into itself for
/// any nested user type at any depth. Never reached for a constructor that
/// carries its own `#[ply::requires]` filter or a fallible (`Result<Self,
/// E>`) return: [`RustType::is_fuzz_nestable`] refuses those *when nested*
/// before this is ever called, because there is no proptest case-rejection
/// available down here (see that method's own doc for why) -- this
/// function only ever needs to render the two shapes nesting actually
/// admits: an infallible constructor call, and a field/variant literal.
fn construct_from_raw_expr(ty: &RustType, var: &str) -> String {
    match ty {
        RustType::UserTypeCtor(plan) => {
            let names: Vec<String> = plan.ctor_params.iter().map(|p| p.name.clone()).collect();
            let bindings = tuple_field_bindings(&names, var);
            let ctor_call = harness::last_two_segments(&plan.constructor);
            let args: Vec<String> = plan
                .ctor_params
                .iter()
                .zip(&bindings)
                .map(|(p, raw)| {
                    let built = construct_from_raw_expr(&p.ty, raw);
                    if p.by_ref { format!("&{built}") } else { built }
                })
                .collect();
            format!("{ctor_call}({})", args.join(", "))
        }
        RustType::UserTypeFields(plan) => match &plan.shape {
            harness::UserTypeShape::Struct(fields) => {
                let names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
                let bindings = tuple_field_bindings(&names, var);
                let inits: Vec<String> = fields
                    .iter()
                    .zip(&bindings)
                    .map(|(f, raw)| format!("{}: {}", f.name, construct_from_raw_expr(&f.ty, raw)))
                    .collect();
                format!("{} {{ {} }}", plan.type_name, inits.join(", "))
            }
            harness::UserTypeShape::Enum(variants) => {
                // `var` is `(discriminant, variant0_raw, variant1_raw, ...)`
                // -- match on the discriminant, building only the chosen
                // variant's own value from its own raw slot (the others
                // were drawn but never used, same as the strategy side).
                let mut arms = String::new();
                for (i, (vname, vfields)) in variants.iter().enumerate() {
                    let variant_raw = format!("{var}.{}", i + 1);
                    let body = if vfields.is_empty() {
                        format!("{}::{vname}", plan.type_name)
                    } else {
                        let names: Vec<String> = vfields.iter().map(|f| f.name.clone()).collect();
                        let bindings = tuple_field_bindings(&names, &variant_raw);
                        let inits: Vec<String> = vfields
                            .iter()
                            .zip(&bindings)
                            .map(|(f, raw)| {
                                format!("{}: {}", f.name, construct_from_raw_expr(&f.ty, raw))
                            })
                            .collect();
                        format!("{}::{vname} {{ {} }}", plan.type_name, inits.join(", "))
                    };
                    arms.push_str(&format!("{i} => {body}, "));
                }
                format!(
                    "match {var}.0 {{ {arms}_ => unreachable!(\"enum discriminant is generated \
                     in 0..{}\") }}",
                    variants.len()
                )
            }
        },
        // Everything else: no wrapping was ever applied (`raw_user_type_
        // strategy_expr`'s recursion bottoms out at an ordinary leaf's own
        // `strategy_expr`, which already produces the real value directly),
        // so the raw binding *is* the real one.
        _ => var.to_string(),
    }
}

/// The per-field expressions `var.0`, `var.1`, ... for `names.len()`
/// fields, matching [`tuple_pattern_and_strategy`]'s own 0/1/N shape
/// exactly: a 0-field tuple has nothing to index (never referenced), a
/// 1-field one *is* the value itself (no `.0` -- `tuple_pattern_and_
/// strategy` never wraps a single field in a 1-tuple), and 2+ fields index
/// an ordinary tuple positionally.
fn tuple_field_bindings(names: &[String], var: &str) -> Vec<String> {
    match names.len() {
        0 => vec![],
        1 => vec![var.to_string()],
        n => (0..n).map(|i| format!("{var}.{i}")).collect(),
    }
}

/// Whether `ty` recursively contains a user-defined struct/enum anywhere --
/// used by [`marker_display_expr`] to decide whether `{:?}` is safe to
/// print: nothing here reads whether the user wrote `#[derive(Debug)]`, so
/// a value containing one cannot be assumed `Debug` the way every built-in
/// leaf and container this codegen builds already is.
fn contains_user_type(ty: &RustType) -> bool {
    match ty {
        RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => true,
        RustType::Option(inner)
        | RustType::Array(inner, _)
        | RustType::Vec(inner)
        | RustType::BTreeSet(inner)
        | RustType::Slice(inner)
        | RustType::BoxT(inner) => contains_user_type(inner),
        RustType::Result(ok, err) => contains_user_type(ok) || contains_user_type(err),
        RustType::Tuple(items) => items.iter().any(contains_user_type),
        RustType::BTreeMap(key, value) => contains_user_type(key) || contains_user_type(value),
        _ => false,
    }
}

/// `format!("{:?}", value_expr)`, then escaped exactly the way
/// `RustType::String`'s own arm below already escapes a bare string's
/// content -- the seven characters this hand-rolled `PLY_FUZZED_CEX` wire
/// format is sensitive to (`\`, `;`, `=`, `[`, `]`, `\n`, `\r`), so a nested
/// value that happens to contain one (an embedded `;` inside a `String`
/// composition now reaches, say `Option<String>`, or a `[`/`]` inside a
/// nested collection) can never be mistaken for this format's own field
/// separator or bracket-depth marker. Added 2026-09-02 alongside
/// composition: before this task nothing composed, so nothing but a bare
/// `String` (already escaped, using its own `Display` rather than `Debug`)
/// ever reached this wire format containing a character it cares about.
/// `split_top_level_semicolons` (`engines::fuzz`) is already escape-aware
/// (a `\` before `;`/`[`/`]` does not count toward either), so this is safe
/// to use anywhere `{:?}` was going to be printed regardless of nesting.
fn debug_escaped_marker_expr(value_expr: &str) -> String {
    format!(
        r#"{{ let mut __ply_s = String::new(); for __ply_c in format!("{{:?}}", {value_expr}).chars() {{ match __ply_c {{ '\\' => __ply_s.push_str("\\\\"), ';' => __ply_s.push_str("\\;"), '=' => __ply_s.push_str("\\="), '[' => __ply_s.push_str("\\["), ']' => __ply_s.push_str("\\]"), '\n' => __ply_s.push_str("\\n"), '\r' => __ply_s.push_str("\\r"), __ply_other => __ply_s.push(__ply_other), }} }} __ply_s }}"#
    )
}

/// The Rust expression text that turns a bound variable `var: ty` into a
/// `Display`-able value for the `PLY_FUZZED_CEX` marker line -- a plain
/// variable reference for scalars (their own `Display` impl), or an inline
/// block that joins a collection into `[a,b,c]` text (no spaces, so the
/// decoder's split-on-comma is exact) for `Vec`/`BTreeSet`.
fn marker_display_expr(ty: &RustType, var: &str) -> String {
    // A value that recursively contains a user-defined struct/enum is not
    // guaranteed to implement `Debug` at all -- nothing here reads whether
    // the user wrote `#[derive(Debug)]`. This is purely the human-readable
    // counterexample line (`engines::fuzz::decode_marker_fields` already
    // reports such a shape witness-only, never decoding it into a
    // structured value), so a fixed, honest placeholder is safe here: it
    // says nothing about the value's fields it cannot back up, rather than
    // guessing at a `Debug` impl that may not exist. A *top-level*
    // user-type parameter is excluded from this check -- its own arm below
    // already has a real, precomputed per-field marker string built before
    // its value is constructed, strictly more informative than this
    // placeholder.
    if !matches!(ty, RustType::UserTypeCtor(_) | RustType::UserTypeFields(_))
        && contains_user_type(ty)
    {
        return "\"<value containing a user-defined type, not shown>\".to_string()".to_string();
    }
    match ty {
        // No `Display` impl for any of these; `Debug` is what a reader of
        // the diagnostic wants to see anyway (`Some(3)`, `'x'`, `[1, 2]`).
        // `char` alone needs no escaping (a bare `char` can never itself
        // collide with this wire format -- `format_args`'s own quoting
        // already keeps it inert), but `Option`/`Result`/`[T; N]`/a tuple/
        // `BTreeMap`/`Box` can now nest anything composition reaches
        // (a `String`, another collection, ...), so those six go through
        // `debug_escaped_marker_expr` instead of a bare `{:?}`.
        RustType::Char => format!("format!(\"{{:?}}\", {var})"),
        RustType::Option(_)
        | RustType::Result(..)
        | RustType::Array(..)
        | RustType::Tuple(_)
        | RustType::BTreeMap(_, _)
        | RustType::BoxT(_) => debug_escaped_marker_expr(var),
        RustType::Vec(_) | RustType::VecU8 | RustType::BTreeSet(_) | RustType::Slice(_) => format!(
            "{{ let mut __ply_s = String::from(\"[\"); \
             for (__ply_i, __ply_e) in {var}.iter().enumerate() {{ \
             if __ply_i > 0 {{ __ply_s.push(','); }} \
             __ply_s.push_str(&{elem_expr}); }} \
             __ply_s.push(']'); __ply_s }}",
            elem_expr = debug_escaped_marker_expr("__ply_e")
        ),
        // `Duration`'s own `Display` picks whichever SI unit reads best
        // ("1.5s", "500ms") -- exactly the ambiguity a decoder must not
        // have to resolve. `secs.nanos` (nanos always 9 digits) is exact and
        // trivially split back apart in `engines::fuzz`.
        RustType::Duration => {
            format!("format!(\"{{}}.{{:09}}\", {var}.as_secs(), {var}.subsec_nanos())")
        }
        // `String` needs its own arm, unlike every scalar the default arm
        // below already handles correctly: the marker line itself is a
        // hand-rolled wire format (`PLY_FUZZED_CEX|fn|k1=v1;k2=v2`, parsed
        // by `engines::fuzz::parse_fuzz_marker`/`split_top_level_
        // semicolons`) that reads `;` as the field separator, the first
        // `=` as the name/value separator, and tracks `[`/`]` depth for a
        // collection field -- and, being line-based, would silently
        // truncate at an embedded newline. A *generated* string is
        // Ply's own content, so it can legitimately contain any of those
        // characters (the content decision on `RustType::String`
        // deliberately does not exclude them) -- so this escapes exactly
        // the seven characters the wire format itself is sensitive to
        // (`\`, `;`, `=`, `[`, `]`, `\n`, `\r`) into a two-character
        // backslash form, character by character (never byte by byte, so
        // a multi-byte character is never split). `engines::fuzz::
        // unescape_marker_value` is the exact, sole reverse of this
        // encoding -- the two must change together. This is the *wire*
        // encoding, never the witness: `String` is not
        // `is_witness_renderable`, so a failure on one is always reported
        // via the raw (here: already-unescaped-by-the-decoder) field text,
        // never a fabricated Rust literal -- see that type's own doc.
        RustType::String => format!(
            r#"{{ let mut __ply_s = String::new(); for __ply_c in {var}.chars() {{ match __ply_c {{ '\\' => __ply_s.push_str("\\\\"), ';' => __ply_s.push_str("\\;"), '=' => __ply_s.push_str("\\="), '[' => __ply_s.push_str("\\["), ']' => __ply_s.push_str("\\]"), '\n' => __ply_s.push_str("\\n"), '\r' => __ply_s.push_str("\\r"), __ply_other => __ply_s.push(__ply_other), }} }} __ply_s }}"#
        ),
        // A struct/enum parameter's own display text is precomputed *before*
        // its value is built (`build_user_value_stmt`'s own doc: the
        // constructor call, or the field/variant literal, may move a
        // leaf's value that the marker also wants to show) into
        // `__ply_marker_val_{var}` -- a `String` already bound by the time
        // the generic `marker_precompute` loop in `generate_fuzz_test`
        // reaches this parameter, so the expression it needs is just that
        // binding's own name, re-`Display`ed (a harmless self-shadowing
        // rewrap, not a second computation). Never `{:?}` of the built
        // value itself: the user's own struct/enum is not guaranteed to
        // derive `Debug` at all, and this must never be the reason a
        // generated harness fails to compile.
        RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => {
            format!("__ply_marker_val_{var}")
        }
        // A seedable-wrapped shape (`Option<String>`/`Vec<String>`) has no
        // `Display` impl either (neither does `Option`/`Vec` of anything),
        // but both derive `Debug` via `String`'s own -- the same `{:?}` the
        // `Option(_)`/`Vec(_)` arms above already use for their own inner
        // types.
        RustType::Unsupported(t) if classify_seedable_wrap(t).is_some() => {
            format!("format!(\"{{:?}}\", {var})")
        }
        // `NonZero{X}`'s own `Display` is already just the plain number
        // (std impls it as a pass-through to the inner integer), so the
        // default arm below is exact for it too -- no case needed.
        _ => var.to_string(),
    }
}

/// The closure/loop-bound pattern proptest's `TestRunner::run` needs to
/// destructure the strategy's produced value: a bare name for one param, a
/// parenthesized tuple pattern for more than one (matching the tuple
/// strategy `combined_strategy_expr_for` builds for that case). Generalised
/// from `cf.params` alone (2026-08-27, receiver construction) so the same
/// logic renders a constructor's own parameter list, or a pooled operation's,
/// without a second copy of the 0/1/N cases.
fn value_pattern_for(params: &[Param]) -> String {
    // Routed through `plan_for_param` (2026-08-27, struct/enum parameters)
    // so a struct/enum Ply itself builds contributes its own nested tuple
    // of synthetic leaf names here instead of its own (unbound) bare name
    // -- byte-identical to the old bare-name-per-param logic for every
    // ordinary type, since `plan_for_param`'s default arm reproduces it
    // exactly. `.expect` is safe: this fn returns no error path of its own
    // (unlike `combined_strategy_expr_for`, which can fail on an
    // `Unsupported` type), and every real caller already gated on
    // `is_fuzz_supported` before reaching here.
    let names: Vec<String> = params
        .iter()
        .map(|p| plan_for_param(p).map(|plan| plan.pattern))
        .collect::<Result<_>>()
        .unwrap_or_else(|e| panic!("value_pattern_for: {e}"));
    match names.len() {
        // A zero-parameter fn (a receiverless constructor like
        // `FakeClock::new()`) has nothing for proptest to generate --
        // `combined_strategy_expr_for` pairs this with `Just(())`, so the
        // pattern that destructures it is `_`, never the empty-tuple
        // pattern `()` the old `else` branch produced (which only ever
        // matched a *value* of `()`, and the strategy below did not build
        // one -- see that fn's own doc for the compile error this caused).
        0 => "_".to_string(),
        1 => names[0].clone(),
        _ => format!("({})", names.join(", ")),
    }
}

/// A unique, collision-free variable-name prefix for pooled operation `i`
/// inside the receiver sequence's own per-step tuple (2026-08-27,
/// docs/review-caveats.md N3: the pool is no longer restricted to the
/// checked method's own parameter shape, so two different pooled
/// operations -- or one of them and the checked method itself -- can
/// otherwise share a parameter *name*). Every operation but the checked
/// method's own repeat (`i == 0`, which keeps its bare names -- see
/// `receiver_preamble`'s doc for why) is prefixed with this before its
/// pattern or its call arguments are rendered.
fn op_prefix(i: usize) -> String {
    format!("__ply_op{i}_")
}

/// [`call_args_for`] reading the same prefixed names
/// [`value_pattern_for_prefixed`] bound.
fn call_args_for_prefixed(params: &[Param], i: usize) -> Vec<String> {
    let prefix = op_prefix(i);
    params
        .iter()
        .map(|p| {
            let name = format!("{prefix}{}", p.name);
            if p.by_ref { format!("&{name}") } else { name }
        })
        .collect()
}

fn value_pattern(cf: &ContractFn) -> String {
    value_pattern_for(&cf.params)
}

/// One raw scalar-ish strategy slot a struct/enum parameter's own build
/// needs -- a constructor argument, a struct field, an enum field, or the
/// discriminant that picks which variant to build -- named uniquely across
/// the whole generated test (`path_prefix`, threaded down through
/// [`build_user_value_stmt`]'s own recursion) so two different built
/// parameters, or two arguments sharing a name at different nesting levels,
/// never collide in the one flat outer tuple every leaf is drawn from.
struct LeafSlot {
    name: String,
    ty: RustType,
    strategy: String,
}

/// Struct/enum parameters (docs/review-self-construction.md's rule, applied
/// to a parameter -- see `harness.rs`'s own module doc on
/// `resolve_user_type` for the three-rule order this renders): recursively
/// builds the imperative Rust snippet that binds `binding_name` to a value
/// of `ty`, appending every leaf strategy slot it draws on to `leaves` and
/// every statement it needs to `preamble`. This is `receiver_preamble`'s
/// exact mechanism (a constructor call gated by its own `#[ply::requires]`;
/// for the direct-construction rule, a struct literal or an enum `match`
/// over a drawn discriminant) generalised from "always `__ply_receiver`" to
/// "whatever name the caller wants this value bound to" -- so a
/// constructor argument or a field that is itself another user type (
/// `Quota::new`'s own `refill: RefillRate` argument, in the rate-limiter
/// fixture) recurses into its own nested block, bound under its **own
/// original** parameter/field name (never a synthetic one), so that type's
/// own `#[ply::requires]` text -- which names its own parameters verbatim
/// -- still resolves correctly by ordinary Rust block scoping. Every
/// *leaf* still gets a globally-unique synthetic name (`path_prefix`
/// accumulates one path segment per nesting level), so the flat outer
/// tuple pattern/strategy never collides even when two different built
/// parameters happen to share a field name.
///
/// An enum (`UserTypeShape::Enum`) always draws **every** variant's own
/// fields, whichever one a discriminant leaf ends up selecting -- simpler
/// than a strategy that only samples the chosen variant's own fields, at
/// the cost of a few wasted draws per case, which proptest does not notice.
fn build_user_value_stmt(
    ty: &RustType,
    binding_name: &str,
    path_prefix: &str,
    leaves: &mut Vec<LeafSlot>,
    preamble: &mut String,
) -> Result<()> {
    match ty {
        RustType::UserTypeCtor(plan) => {
            preamble.push_str(&format!("            let {binding_name} = {{\n"));
            for cp in &plan.ctor_params {
                let sub_prefix = format!("{path_prefix}{}_", cp.name);
                build_user_value_stmt(&cp.ty, &cp.name, &sub_prefix, leaves, preamble)?;
            }
            if let Some(req) = &plan.ctor_requires {
                let cond = req.to_token_stream().to_string();
                preamble.push_str(&format!(
                    "                if !({cond}) {{ __ply_rejected.set(__ply_rejected.get() + \
                     1); return Err(proptest::test_runner::TestCaseError::reject(\"constructor \
                     requires filter\")); }}\n"
                ));
            }
            let ctor_call = harness::last_two_segments(&plan.constructor);
            let ctor_args = call_args_for(&plan.ctor_params).join(", ");
            // A fallible constructor (docs/review-structs-enums.md finding
            // 2, "a violation reported on correct code", 2026-08-28): a
            // `Result<Self, E>`-returning `new` rejects some inputs by
            // design (`Range::new(lo, hi)` when `lo > hi`), and an `Err` is
            // not a usable value -- the case that produced it is discarded,
            // the same way an unsatisfied `requires` is, never unwrapped
            // (which would panic on Ply's own generated code, reported as
            // the checked function's fault) and never treated as though the
            // `Err` payload were a `Self`.
            match plan.ctor_return {
                harness::CtorReturn::Bare => {
                    preamble.push_str(&format!(
                        "                {ctor_call}({ctor_args})\n            }};\n"
                    ));
                }
                harness::CtorReturn::ResultSelf => {
                    preamble.push_str(&format!(
                        "                match {ctor_call}({ctor_args}) {{\n                    \
                         Ok(__ply_ctor_ok) => __ply_ctor_ok,\n                    Err(_) => {{ \
                         __ply_rejected.set(__ply_rejected.get() + 1); return \
                         Err(proptest::test_runner::TestCaseError::reject(\"constructor \
                         returned Err\")); }}\n                }}\n            }};\n"
                    ));
                }
            }
            Ok(())
        }
        RustType::UserTypeFields(plan) => match &plan.shape {
            harness::UserTypeShape::Struct(fields) => {
                preamble.push_str(&format!("            let {binding_name} = {{\n"));
                for f in fields {
                    let sub_prefix = format!("{path_prefix}{}_", f.name);
                    build_user_value_stmt(&f.ty, &f.name, &sub_prefix, leaves, preamble)?;
                }
                let field_inits: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
                preamble.push_str(&format!(
                    "                {} {{ {} }}\n            }};\n",
                    plan.type_name,
                    field_inits.join(", ")
                ));
                Ok(())
            }
            harness::UserTypeShape::Enum(variants) => {
                let disc_name = format!("__ply_leaf_{path_prefix}variant");
                leaves.push(LeafSlot {
                    name: disc_name.clone(),
                    ty: RustType::U8,
                    strategy: format!("0u8..{}u8", variants.len()),
                });
                preamble.push_str(&format!("            let {binding_name} = {{\n"));
                preamble.push_str(&format!("                match {disc_name} {{\n"));
                for (i, (vname, vfields)) in variants.iter().enumerate() {
                    preamble.push_str(&format!("                    {i} => {{\n"));
                    for f in vfields {
                        let sub_prefix = format!("{path_prefix}v{i}_{}_", f.name);
                        build_user_value_stmt(&f.ty, &f.name, &sub_prefix, leaves, preamble)?;
                    }
                    if vfields.is_empty() {
                        preamble.push_str(&format!(
                            "                        {}::{vname}\n                    }}\n",
                            plan.type_name
                        ));
                    } else {
                        let field_inits: Vec<String> =
                            vfields.iter().map(|f| f.name.clone()).collect();
                        preamble.push_str(&format!(
                            "                        {}::{vname} {{ {} }}\n                    }}\n",
                            plan.type_name,
                            field_inits.join(", ")
                        ));
                    }
                }
                preamble.push_str(&format!(
                    "                    _ => unreachable!(\"{disc_name} is generated in \
                     0..{}\"),\n",
                    variants.len()
                ));
                preamble.push_str("                }\n            };\n");
                Ok(())
            }
        },
        _ => {
            let leaf_name = format!("__ply_leaf_{path_prefix}{binding_name}");
            leaves.push(LeafSlot {
                name: leaf_name.clone(),
                ty: ty.clone(),
                strategy: strategy_expr(ty)?,
            });
            preamble.push_str(&format!("            let {binding_name} = {leaf_name};\n"));
            Ok(())
        }
    }
}

/// The `PLY_FUZZED_CEX` marker text for a struct/enum parameter, precomputed
/// **before** its own construction preamble runs (`build_user_value_stmt`'s
/// own doc: the constructor call, or the field/variant literal, may move a
/// leaf's value the marker also wants to show) -- built from each leaf's
/// own already-decodable `marker_display_expr`, since every leaf is by
/// construction an ordinary type, never another unresolved user type.
fn build_marker_stmt(param_name: &str, leaves: &[LeafSlot]) -> String {
    if leaves.is_empty() {
        return format!("            let __ply_marker_val_{param_name}: String = String::new();\n");
    }
    let fmt_str = leaves
        .iter()
        .map(|l| format!("{}={{}}", l.name))
        .collect::<Vec<_>>()
        .join(", ");
    let args = leaves
        .iter()
        .map(|l| marker_display_expr(&l.ty, &l.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "            let __ply_marker_val_{param_name}: String = format!(\"{fmt_str}\", {args});\n"
    )
}

/// Everything needed to fold one parameter into the outer combined proptest
/// tuple: the pattern fragment (a bare name for an ordinary type; a nested
/// tuple of synthetic leaf names for a struct/enum Ply itself builds), the
/// matching strategy fragment, and the preamble text (empty for an
/// ordinary type) that turns those leaves into the parameter's own bound
/// name before the checked call runs. For an ordinary type this is
/// byte-for-byte what the pre-existing per-param logic already produced --
/// no behaviour changes for any function with no struct/enum parameter.
struct ParamPlan {
    pattern: String,
    strategy: String,
    preamble: String,
}

fn plan_for_param(p: &Param) -> Result<ParamPlan> {
    match &p.ty {
        RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => {
            let mut leaves = Vec::new();
            let mut preamble = String::new();
            build_user_value_stmt(
                &p.ty,
                &p.name,
                &format!("p_{}_", p.name),
                &mut leaves,
                &mut preamble,
            )?;
            let pattern = match leaves.len() {
                0 => "_".to_string(),
                1 => leaves[0].name.clone(),
                _ => format!(
                    "({})",
                    leaves
                        .iter()
                        .map(|l| l.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            let strategy = match leaves.len() {
                0 => "proptest::strategy::Just(())".to_string(),
                1 => leaves[0].strategy.clone(),
                _ => format!(
                    "({})",
                    leaves
                        .iter()
                        .map(|l| l.strategy.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            let marker_stmt = build_marker_stmt(&p.name, &leaves);
            Ok(ParamPlan {
                pattern,
                strategy,
                preamble: format!("{marker_stmt}{preamble}"),
            })
        }
        // Composition (2026-09-02, TODO.md): `p.ty` is not itself a
        // struct/enum, but nests one somewhere (`Option<Doc>`, `Vec<Doc>`,
        // ...) -- `strategy_expr` already builds it, wrapped in
        // `__PlyOpaque` wherever it reaches the nested type (see
        // `wrap_fn_harness_module`'s own doc for why), so the value bound
        // to `p.name` here still needs one preamble statement to strip that
        // wrapper back off before the real function ever sees it.
        ty if contains_user_type(ty) => {
            let raw_name = format!("__ply_raw_{}", p.name);
            Ok(ParamPlan {
                pattern: raw_name.clone(),
                strategy: strategy_expr(ty)?,
                preamble: format!(
                    "            let {name} = {unwrap};\n",
                    name = p.name,
                    unwrap = unwrap_composed_expr(ty, &raw_name)
                ),
            })
        }
        _ => Ok(ParamPlan {
            pattern: p.name.clone(),
            strategy: strategy_expr(&p.ty)?,
            preamble: String::new(),
        }),
    }
}

/// The container-level reverse of [`raw_user_type_strategy_expr`]'s own
/// recursion into `strategy_expr` -- an expression of `ty`'s own real type,
/// built from `var` (whose actual runtime type is `ty` with every nested
/// struct/enum replaced by its raw leaf tuple). Recurses the same way
/// `strategy_expr`'s own composing arms do, so the conversion reaches a
/// nested type at any depth (a `Doc` two levels down inside
/// `Vec<Option<Doc>>`, say), stopping the instant a branch contains no user
/// type at all (`var` unchanged from there down -- nothing was ever
/// replaced, so there is nothing to convert).
fn unwrap_composed_expr(ty: &RustType, var: &str) -> String {
    if !contains_user_type(ty) {
        return var.to_string();
    }
    match ty {
        RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => construct_from_raw_expr(ty, var),
        RustType::Option(inner) => format!(
            "{var}.map(|__ply_v| {})",
            unwrap_composed_expr(inner, "__ply_v")
        ),
        RustType::Result(ok, err) => format!(
            "{var}.map(|__ply_v| {}).map_err(|__ply_v| {})",
            unwrap_composed_expr(ok, "__ply_v"),
            unwrap_composed_expr(err, "__ply_v"),
        ),
        RustType::Array(inner, _) => format!(
            "{var}.map(|__ply_v| {})",
            unwrap_composed_expr(inner, "__ply_v")
        ),
        RustType::Vec(inner) | RustType::BTreeSet(inner) => format!(
            "{var}.into_iter().map(|__ply_v| {}).collect()",
            unwrap_composed_expr(inner, "__ply_v")
        ),
        // A slice needs the collection named, where `Vec` and `BTreeSet` do
        // not: their bindings are annotated by the parameter's own type, but
        // a `&[T]` parameter is lent from an owned value whose type nothing
        // else states. Left to inference, the call site decided it -- and
        // the call site wants `&[T]`, so the binding inferred to the unsized
        // `[T]` and the harness would not compile. Shipped that way with
        // slice support on 2026-09-02, and found by pointing Ply at its own
        // code: one function taking `&[HarnessModule]` took every claim in
        // the crate down with it, because they share one harness.
        RustType::Slice(inner) => format!(
            "{var}.into_iter().map(|__ply_v| {}).collect::<Vec<_>>()",
            unwrap_composed_expr(inner, "__ply_v")
        ),
        RustType::Tuple(items) => {
            let parts: Vec<String> = items
                .iter()
                .enumerate()
                .map(|(i, t)| unwrap_composed_expr(t, &format!("{var}.{i}")))
                .collect();
            match parts.len() {
                0 => "()".to_string(),
                1 => format!("({},)", parts[0]),
                _ => format!("({})", parts.join(", ")),
            }
        }
        RustType::BTreeMap(key, value) => format!(
            "{var}.into_iter().map(|(__ply_k, __ply_v)| ({}, {})).collect()",
            unwrap_composed_expr(key, "__ply_k"),
            unwrap_composed_expr(value, "__ply_v"),
        ),
        RustType::BoxT(inner) => format!(
            "Box::new({})",
            unwrap_composed_expr(inner, &format!("*{var}"))
        ),
        _ => var.to_string(),
    }
}

/// The preamble text every built parameter in `params` contributes, in
/// order -- empty when none of them is a struct/enum Ply itself builds.
fn params_preamble(params: &[Param]) -> Result<String> {
    let plans: Vec<ParamPlan> = params.iter().map(plan_for_param).collect::<Result<_>>()?;
    Ok(plans.into_iter().map(|p| p.preamble).collect())
}

fn combined_strategy_expr_for(params: &[Param]) -> Result<String> {
    let plans: Vec<ParamPlan> = params.iter().map(plan_for_param).collect::<Result<_>>()?;
    Ok(match plans.len() {
        // A zero-parameter fn (found broken 2026-08-27 against
        // `Meter::zero`/`FakeClock::new`-shaped constructors, adversarial
        // review of the method-resolution task): the old `else` branch
        // joined zero expressions into a bare `()`, a *value*, not a
        // `Strategy` -- `TestRunner::run` needs `&impl Strategy`, so every
        // zero-param fuzz claim failed to compile with "the trait bound
        // `(): Strategy` is not satisfied" (`X0901`), regardless of its
        // return type. `Just(())` is proptest's own always-`()` strategy,
        // built for exactly this case.
        0 => "proptest::strategy::Just(())".to_string(),
        1 => plans[0].strategy.clone(),
        _ => format!(
            "({})",
            plans
                .iter()
                .map(|p| p.strategy.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

fn combined_strategy_expr(cf: &ContractFn) -> Result<String> {
    combined_strategy_expr_for(&cf.params)
}

/// Exactly [`combined_strategy_expr_for`], except the named parameter's own
/// strategy fragment is replaced with `strategy_expr_override` wholesale --
/// used only to splice the corpus-backed seeded strategy into one
/// constructor parameter's slot (`receiver_pattern_and_strategy`) without
/// duplicating the tuple-building logic above it. `override_slot: None`
/// (every constructor with nothing seeded) computes byte-identical output
/// to `combined_strategy_expr_for`.
fn combined_strategy_expr_for_with_override(
    params: &[Param],
    override_slot: Option<(&str, &str)>,
) -> Result<String> {
    let plans: Vec<ParamPlan> = params.iter().map(plan_for_param).collect::<Result<_>>()?;
    let mut strategies: Vec<String> = plans.into_iter().map(|p| p.strategy).collect();
    if let Some((name, expr)) = override_slot
        && let Some(idx) = params.iter().position(|p| p.name == name)
    {
        strategies[idx] = expr.to_string();
    }
    Ok(match strategies.len() {
        0 => "proptest::strategy::Just(())".to_string(),
        1 => strategies[0].clone(),
        _ => format!("({})", strategies.join(", ")),
    })
}

fn call_args_for(params: &[Param]) -> Vec<String> {
    params
        .iter()
        .map(|p| {
            if p.by_ref {
                format!("&{}", p.name)
            } else {
                p.name.clone()
            }
        })
        .collect()
}

fn call_args(cf: &ContractFn) -> Vec<String> {
    call_args_for(&cf.params)
}

/// True for a type that *moves* when passed by value -- so a parameter of
/// this type, taken by value (not `&`), no longer exists once the call it
/// was passed to returns. The scalars, `bool` and `char` are `Copy`; so is
/// `Option`/`Result`/`[T; N]` of an inner type that is itself `Copy`
/// (mirrored recursively, matching `derive(Copy)`'s own rule).
fn moves_on_by_value_call(ty: &RustType) -> bool {
    match ty {
        RustType::VecU8 | RustType::Vec(_) | RustType::BTreeSet(_) | RustType::String => true,
        RustType::Option(inner) => moves_on_by_value_call(inner),
        RustType::Result(ok, err) => moves_on_by_value_call(ok) || moves_on_by_value_call(err),
        RustType::Array(inner, _) => moves_on_by_value_call(inner),
        // A seedable-wrapped shape (`Option<String>`/`Vec<String>`, see
        // `SeedableWrap`'s own doc) owns a `String` inside, so it moves on
        // a by-value call exactly as `RustType::String` itself already
        // does above -- any other `Unsupported` shape is opaque to this
        // codegen and conservatively assumed not to move, same as before
        // this task.
        RustType::Unsupported(t) => classify_seedable_wrap(t).is_some(),
        // A struct/enum parameter (2026-08-27): conservative rather than
        // measured -- Ply's parser does not read `#[derive(Copy)]`, so
        // whether a given user type actually moves on a by-value call is
        // genuinely unknown here. Assuming it does is the safe direction:
        // a postcondition that reads it back after the call is refused up
        // front with a specific fix (`V0506`, "wrap the read in `old(...)`,
        // or take it by reference"), instead of risking a raw
        // `error[E0382]: borrow of moved value` inside Ply's own generated
        // harness for the (real, common) case where the type is not
        // actually `Copy`.
        RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => true,
        // Composition (2026-09-02): a tuple moves if any element does
        // (mirroring `derive(Copy)`'s own all-or-nothing rule, same as
        // `Result` just above); `BTreeMap` and `Box` always own heap
        // allocations, so both always move, the same as `Vec`/`BTreeSet`/
        // `String` above. `Slice` is conservatively `true` too, though it
        // is never actually reached here -- a slice parameter is only ever
        // written `&[T]`, so `by_ref` is always set and `moved_names`
        // (this fn's only caller) already filters those out before asking.
        RustType::Tuple(items) => items.iter().any(moves_on_by_value_call),
        RustType::BTreeMap(_, _) | RustType::BoxT(_) | RustType::Slice(_) => true,
        _ => false,
    }
}

/// The first bare reference to a name in `moved_names`, found anywhere in
/// `expr`. Used only after `old(...)` has already been rewritten out of the
/// tree (`contract_rt::lift_entry_values`), so a legitimate `old(v)` read --
/// which evaluates before the call, on the value that still exists -- has
/// already become `__ply_old_0` and cannot match; whatever is left really
/// would be a read of `v` *after* it was moved into the call.
fn find_moved_param_read(expr: &Expr, moved_names: &BTreeSet<&str>) -> Option<String> {
    struct Finder<'a> {
        moved: &'a BTreeSet<&'a str>,
        found: Option<String>,
    }
    impl<'a> Visit<'a> for Finder<'a> {
        fn visit_expr_path(&mut self, node: &'a syn::ExprPath) {
            if self.found.is_some() {
                return;
            }
            if let Some(ident) = node.path.get_ident()
                && self.moved.contains(ident.to_string().as_str())
            {
                self.found = Some(ident.to_string());
                return;
            }
            syn::visit::visit_expr_path(self, node);
        }
    }
    let mut finder = Finder {
        moved: moved_names,
        found: None,
    };
    finder.visit_expr(expr);
    finder.found
}

/// The parameter, if any, that `cf`'s postcondition reads *after* it has
/// already been moved into the call: a by-value (non-`&`), non-`Copy`
/// parameter no longer exists once `fname(...)` returns, so a generated
/// harness that reads it there is `error[E0382]: borrow of moved value` --
/// not a bug in the generated code, but a contract shape Ply cannot render
/// at all. `old(param)` is exempt on purpose: it captures `param`'s value
/// *before* the call (`contract_rt::lift_entry_values` already rewrites
/// every `old(...)` occurrence into its own pre-call binding, so what
/// reaches this function's scan is exactly what is left over -- a bare read
/// with no `old()` around it, which can only mean after the call).
pub fn moved_param_read_in_ensures(cf: &ContractFn) -> Option<&Param> {
    let (closure, _) = cf.ensures.as_ref()?;
    let moved_names: BTreeSet<&str> = cf
        .params
        .iter()
        .filter(|p| !p.by_ref && moves_on_by_value_call(&p.ty))
        .map(|p| p.name.as_str())
        .collect();
    if moved_names.is_empty() {
        return None;
    }
    let (checked_body, _) = crate::contract_rt::lift_entry_values(&closure.body);
    let found_name = find_moved_param_read(&checked_body, &moved_names)?;
    cf.params.iter().find(|p| p.name == found_name)
}

/// [`plan_for_param`] for one operation's arguments, under that step's own
/// name prefix.
///
/// Every name `plan_for_param` mints is derived from the parameter's own
/// name, so prefixing the name is enough to prefix the whole plan --
/// pattern, generated leaves, marker, and the `let` the preamble binds. For
/// an ordinary scalar argument this is byte-identical to the plain prefixed
/// pattern that came before it; what it adds is the case that argument
/// could not have: an argument Ply has to *build* (a struct, or a plain
/// enum) needs a `let` statement, and the sequence loop had no way to emit
/// one. That gap is why a mutator taking a plain enum was dropped from the
/// pool entirely, while the identical enum built fine one line away as an
/// ordinary parameter (TODO.md, 2026-09-03).
fn plans_for_op_params(params: &[Param], i: usize) -> Result<Vec<ParamPlan>> {
    let prefix = op_prefix(i);
    params
        .iter()
        .map(|p| {
            plan_for_param(&Param {
                name: format!("{prefix}{}", p.name),
                ..p.clone()
            })
        })
        .collect()
}

/// The pattern, strategy and preamble one operation's arguments need, joined
/// under proptest's own tuple rules -- the same 0/1/many shape
/// [`combined_strategy_expr_for`] follows, because a one-element tuple is not
/// a `Strategy` and an empty one is not a value.
fn op_pattern_strategy_preamble(params: &[Param], i: usize) -> Result<(String, String, String)> {
    let plans = plans_for_op_params(params, i)?;
    let join = |parts: Vec<String>, empty: &str| match parts.len() {
        0 => empty.to_string(),
        1 => parts[0].clone(),
        _ => format!("({})", parts.join(", ")),
    };
    let pattern = join(plans.iter().map(|p| p.pattern.clone()).collect(), "_");
    let strategy = join(
        plans.iter().map(|p| p.strategy.clone()).collect(),
        "proptest::strategy::Just(())",
    );
    let preamble = plans.iter().map(|p| p.preamble.clone()).collect::<String>();
    Ok((pattern, strategy, preamble))
}
/// Every *other* pooled operation gets its own strategy and its own
/// (prefixed) pattern (2026-08-27, docs/review-caveats.md N3): the pool is
/// no longer restricted to the checked method's own parameter shape, so a
/// mixed-shape step needs its own slot per operation rather than one shared
/// one.
/// The receiver half of a generated fuzz test (docs/review-self-construction.md's
/// "fourth option", 2026-08-27): the outer strategy/pattern grow a leading
/// constructor slot and a bounded-sequence slot, and the closure body grows a
/// preamble that builds the receiver and drives the sequence, before the
/// checked call runs -- exactly the shape a stateful-property test always
/// has, generated instead of hand-written.
///
/// `target_pattern`/`target_strategy` are the checked method's *own*
/// (already-computed) pattern and strategy, reused verbatim as operation
/// zero's own slot in the per-step tuple below -- the checked method is
/// always pooled (`ReceiverPlan::operations[0]`), and giving its repeat the
/// same bare names as the final call is what lets its own `#[ply::requires]`
/// text be spliced into the loop unmodified (`receiver_preamble`'s doc).
fn receiver_pattern_and_strategy(
    plan: &harness::ReceiverPlan,
    target_pattern: &str,
    target_strategy: &str,
    seed_plan: Option<&ReceiverSeedPlan>,
) -> Result<(String, String)> {
    let ctor_pattern = value_pattern_for(&plan.ctor_params);
    // Seeded generation (docs/reach-measurement-2.md): when this
    // constructor's text parameter is gated (a `requires`, or a fallible
    // return) and a `ReceiverSeedPlan` was built for it, that one param's
    // own strategy slot is overridden to draw from the corpus-backed
    // `__PlySeedStrategy` (see `seed_apparatus`) instead of uniform text.
    // Every other constructor -- the overwhelming majority -- takes the
    // `None` arm below, byte-identical to before this existed.
    let ctor_strategy = match seed_plan {
        Some(sp) => combined_strategy_expr_for_with_override(
            &plan.ctor_params,
            Some((
                &sp.param_name,
                "__PlySeedStrategy { corpus: __ply_seed_corpus.clone() }",
            )),
        )?,
        None => combined_strategy_expr_for(&plan.ctor_params)?,
    };
    let num_ops = plan.operations.len();
    let mut step_strategies = vec![target_strategy.to_string()];
    for (i, op) in plan.operations.iter().enumerate().skip(1) {
        step_strategies.push(op_pattern_strategy_preamble(&op.params, i)?.1);
    }
    let seq_strategy = format!(
        "proptest::collection::vec((0u8..{num_ops}u8, {}), 0..={max}usize)",
        step_strategies.join(", "),
        max = plan.max_sequence_len
    );
    let pattern = format!("({ctor_pattern}, __ply_seq, {target_pattern})");
    let strategy = format!("({ctor_strategy}, {seq_strategy}, {target_strategy})");
    Ok((pattern, strategy))
}

/// The preamble text inserted into the generated test, after `requires` has
/// already rejected an unsuitable case and before the checked call: builds
/// the receiver by calling the type's own constructor
/// (`docs/review-self-construction.md`'s whole point -- never a struct
/// literal, never a field), then runs up to `plan.max_sequence_len` of the
/// type's own operations against it, each with its own freshly generated
/// arguments.
///
/// Two honesty fixes land here alongside the mixed-shape pool
/// (2026-08-27, docs/review-caveats.md N2 -- "Ply ignores the rules the
/// type itself declares about how that value may be built"):
///
/// - the constructor's own `#[ply::requires]`, if it declares one, gates
///   the arguments generated for it, exactly like the checked call's own
///   `requires` already gates its arguments -- a violated constructor
///   precondition used to panic on entry, and that panic was reported as
///   the *checked method* breaking its own promise;
/// - the checked method's own `#[ply::requires]` gates *every* call the
///   sequence makes to it, not only the final one: operation zero is
///   always a repeat of the checked method (`ReceiverPlan::operations`),
///   and before this fix only the outer, final call's arguments were
///   filtered -- an earlier repeat inside the loop drew its arguments
///   unfiltered, so calling the checked method out of its own contract
///   from *inside* the sequence was exactly how a violation used to be
///   reported on correct code. A step whose drawn arguments fail that
///   check is skipped, not attempted, never rejecting the whole case (the
///   checked method's own precondition says nothing about which other
///   states are reachable, only that this particular call would not be a
///   real one).
fn receiver_preamble(
    cf: &ContractFn,
    plan: &harness::ReceiverPlan,
    target_pattern: &str,
    seed_plan: Option<&ReceiverSeedPlan>,
) -> Result<String> {
    let ctor_call = harness::last_two_segments(&plan.constructor);
    let ctor_args = call_args_for(&plan.ctor_params).join(", ");

    let mut body = String::new();
    // Struct/enum parameters (2026-08-27): one of the receiver's own
    // constructor arguments may itself be another user type Ply builds
    // (`Quota::new`'s own `refill: RefillRate` argument, in the rate-limiter
    // fixture) -- its own construction preamble must run, and its own value
    // must be bound under its original name, before `ctor_args` above (which
    // references that same name) is ever spliced into the ctor call below.
    // A plain ctor argument contributes nothing here (`plan_for_param`'s
    // default arm), so this is a no-op for every receiver this task did not
    // touch.
    body.push_str(&params_preamble(&plan.ctor_params)?);
    if let Some(ctor_requires) = &plan.ctor_requires {
        let cond = ctor_requires.to_token_stream().to_string();
        body.push_str(&format!(
            "if !({cond}) {{ __ply_rejected.set(__ply_rejected.get() + 1); \
             return Err(proptest::test_runner::TestCaseError::reject(\"constructor requires \
             filter\")); }}\n            "
        ));
    }
    let needs_mut = plan.operations.iter().any(|op| op.takes_mut_self);
    let mut_kw = if needs_mut { "mut " } else { "" };
    // A fallible constructor (defect 1, 2026-08-31, docs/reach-measurement-2.md):
    // `plan.ctor_return` now carries the real shape (see `ReceiverPlan::ctor_return`'s
    // own doc), so a `Result<Self, E>`-returning constructor gets the same
    // rejecting `match` `build_user_value_stmt` already renders for the
    // parameter path, never unwrapped and never treated as though the `Err`
    // payload were a receiver.
    let ctor_expr = match plan.ctor_return {
        harness::CtorReturn::Bare => format!("{ctor_call}({ctor_args})"),
        harness::CtorReturn::ResultSelf => format!(
            "match {ctor_call}({ctor_args}) {{\n                Ok(__ply_ctor_ok) => \
             __ply_ctor_ok,\n                Err(_) => {{ __ply_rejected.set(__ply_rejected.get() \
             + 1); return Err(proptest::test_runner::TestCaseError::reject(\"constructor returned \
             Err\")); }}\n            }}"
        ),
    };
    body.push_str(&format!(
        "let {mut_kw}__ply_receiver = {ctor_expr};\n            "
    ));
    // Source 2 (design brief, docs/reach-measurement-2.md): reaching this
    // line means every gate above already passed -- the requires filter, if
    // any, and the constructor's own `Result`, if fallible -- so the text
    // that was just accepted is certified valid by the code under check
    // itself. It joins the corpus for the rest of *this* run, which is why
    // a seeded run's evidence keeps improving as it goes rather than
    // staying pinned to whatever `examples:` alone provided.
    if let Some(sp) = seed_plan {
        body.push_str(&format!(
            "__ply_seed_corpus.borrow_mut().push({}.clone()); \
             __ply_seed_corpus_grown.set(__ply_seed_corpus_grown.get() + 1);\n            ",
            sp.param_name
        ));
    }

    // Built once and reused by both halves below, so the pattern the loop
    // destructures and the `let` bindings inside each arm can never drift
    // apart -- they are two views of the same plan.
    let op_plans: Vec<(String, String)> = plan
        .operations
        .iter()
        .enumerate()
        .map(|(i, op)| {
            if i == 0 {
                Ok((target_pattern.to_string(), String::new()))
            } else {
                let (pattern, _, preamble) = op_pattern_strategy_preamble(&op.params, i)?;
                Ok((pattern, preamble))
            }
        })
        .collect::<Result<_>>()?;
    let step_pattern = op_plans
        .iter()
        .map(|(pattern, _)| pattern.clone())
        .collect::<Vec<_>>()
        .join(", ");
    body.push_str(&format!(
        "for (__ply_op_choice, {step_pattern}) in __ply_seq {{\n"
    ));
    body.push_str("                match __ply_op_choice {\n");
    for (i, op) in plan.operations.iter().enumerate() {
        let call = harness::last_two_segments(&op.call_path);
        let op_args = if i == 0 {
            call_args_for(&op.params).join(", ")
        } else {
            call_args_for_prefixed(&op.params, i).join(", ")
        };
        let recv_ref = if op.takes_mut_self {
            "&mut __ply_receiver"
        } else {
            "&__ply_receiver"
        };
        let full_args = if op_args.is_empty() {
            recv_ref.to_string()
        } else {
            format!("{recv_ref}, {op_args}")
        };
        // An argument Ply builds rather than draws needs its `let` to run
        // inside this arm, before the call that reads it -- the piece the
        // sequence loop never had, and the whole reason an operation taking
        // one was left out of the pool instead of called.
        let bind = &op_plans[i].1;
        let call_stmt = format!("{bind}let _ = {call}({full_args});");
        let arm_body = if i == 0 {
            match &cf.requires {
                Some((expr, _)) => {
                    let cond = expr.to_token_stream().to_string();
                    format!("if {cond} {{ {call_stmt} }}")
                }
                None => call_stmt,
            }
        } else {
            call_stmt
        };
        body.push_str(&format!("                    {i} => {{ {arm_body} }}\n"));
    }
    body.push_str(
        "                    _ => unreachable!(\"__ply_op_choice is generated in 0..num_ops\"),\n",
    );
    body.push_str("                }\n            }\n            ");
    Ok(body)
}

// -- Seeded generation (docs/reach-measurement-2.md: "a type built from
// text cannot be constructed from random text"). A receiver whose own
// constructor parses a `&str`/`String` and is gated (a `#[ply::requires]`,
// or a fallible `Result<Self, E>` return) grows a corpus of known-valid
// text -- literal arguments a user's `examples:` already pass to that
// constructor, plus every value the constructor accepts during the run --
// and draws future cases as a mix of mutations of that corpus alongside a
// continuing uniform trickle, rather than uniform text alone. See
// `plan_receiver_seeding` for when this applies at all (an ungated
// constructor is never touched) and `seed_apparatus` for the generated
// runtime support.

/// 4:1, mutate-from-corpus to uniform-trickle, once the corpus holds at
/// least one known-valid value (empty corpus always trickles -- there is
/// nothing yet to mutate). This is a real design parameter, not a footnote:
/// the measured baseline (`docs/reach-measurement-2.md`) was 49 accepted out
/// of 1074 *uniform* draws, roughly 4.6%, so a mix anywhere near uniform
/// would still mostly fail to earn evidence -- the whole point of seeding.
/// A ratio of 1 (all mutation, no trickle) was rejected instead: the brief's
/// own "known failure mode" is that seeds anchor the distribution away from
/// the extremes an author actually cared about, and *dropping* the uniform
/// slice entirely would make a run permanently self-referential -- it could
/// never discover a valid shape the corpus does not already resemble, and a
/// pathological input reachable only by uniform luck would become
/// permanently unreachable rather than merely less likely. Keeping a full
/// fifth of draws genuinely uniform is what keeps the corpus itself capable
/// of growing into new territory, not just mutating around what it started
/// with -- the same shape of trade-off the integer strategies above already
/// make (`prop_oneof![3 => small, 1 => any]`) and the string strategy makes
/// again for its content (`9 => ascii, 1 => unicode`). Recorded here, and
/// named in the diagnostic `verify` emits for a seeded run, so the ratio
/// reaches the JSON envelope rather than living only in this comment.
pub(crate) const SEED_MUTATE_WEIGHT: u32 = 4;
pub(crate) const SEED_TRICKLE_WEIGHT: u32 = 1;

/// What a seeded receiver constructor parameter needs: which parameter
/// (by name, so `receiver_preamble` can splice a read of its own bound
/// variable) and the seeds pulled from `examples:` at codegen time (source
/// 1 of 2 -- source 2, values the constructor accepts at runtime, is grown
/// entirely inside the generated harness and never seen here).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReceiverSeedPlan {
    param_name: String,
    examples_seeds: Vec<String>,
}

/// Whether `plan`'s constructor should be seeded at all, and if so, from
/// what. Deliberately narrow (2026-09-01): seeding only a receiver's own
/// constructor, only its first `String`-typed parameter, and only when that
/// constructor actually rejects something -- an ungated constructor accepts
/// every draw already, so seeding it would be a no-op wearing a feature's
/// clothes, and the honesty condition ("a seeded verdict must never be
/// indistinguishable from an unseeded one") is easiest to keep by simply
/// never seeding where nothing is gated. `None` means every line downstream
/// of this stays byte-identical to before this feature existed.
fn plan_receiver_seeding(
    plan: &harness::ReceiverPlan,
    examples_pool: &[String],
) -> Option<ReceiverSeedPlan> {
    let gated =
        plan.ctor_requires.is_some() || matches!(plan.ctor_return, harness::CtorReturn::ResultSelf);
    if !gated {
        return None;
    }
    let string_param = plan.ctor_params.iter().find(|p| p.ty == RustType::String)?;
    let examples_seeds = extract_examples_seed_strings(examples_pool, &plan.constructor);
    Some(ReceiverSeedPlan {
        param_name: string_param.name.clone(),
        examples_seeds,
    })
}

/// Source 1 (design brief): every string-literal argument passed to a call
/// matching `ctor_path` (matched on its last two `::`-segments, the same
/// convention [`harness::last_two_segments`] uses everywhere else), found
/// anywhere in `examples` -- purely syntactic, exactly like
/// [`generate_example_test`] already parses these same strings into
/// assertions. Zero new vocabulary: an `examples:` entry a user already
/// wrote is scanned for calls to the constructor being seeded, and its
/// literal string arguments become known-valid corpus values. An entry that
/// does not parse contributes nothing here -- it is someone else's
/// diagnostic (`E0501`) to report, not this extractor's.
pub fn extract_examples_seed_strings(examples: &[String], ctor_path: &str) -> Vec<String> {
    struct CtorArgCollector<'a> {
        target: &'a str,
        out: Vec<String>,
    }
    impl<'a> Visit<'a> for CtorArgCollector<'a> {
        fn visit_expr_call(&mut self, node: &'a syn::ExprCall) {
            let func_text = node.func.to_token_stream().to_string().replace(' ', "");
            if harness::last_two_segments(&func_text) == self.target {
                for arg in &node.args {
                    if let Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(s),
                        ..
                    }) = arg
                    {
                        self.out.push(s.value());
                    }
                }
            }
            syn::visit::visit_expr_call(self, node);
        }
    }
    let target = harness::last_two_segments(ctor_path);
    let mut out = Vec::new();
    for example in examples {
        let Ok(expr) = syn::parse_str::<Expr>(example) else {
            continue;
        };
        let mut collector = CtorArgCollector {
            target: &target,
            out: Vec::new(),
        };
        collector.visit_expr(&expr);
        out.append(&mut collector.out);
    }
    out
}

// -- widening past the receiver-constructor case (2026-09-01, TODO.md "an
// example does not unblock a parameter Ply cannot build"). Everything above
// this point seeds a receiver's own constructor; everything below seeds a
// *plain* (non-receiver) function's own parameter whose type `RustType`
// deliberately never builds at all -- `Option<String>`/`Vec<String>` (see
// `SeedableWrap`'s own doc for exactly why these two and not, say, nested
// `NonZero`/`Duration`/`f32`/`f64`, which are real widenings not attempted
// this session). The two mechanisms never coexist on one generated harness
// (`plan_param_seeding` refuses outright whenever `cf.receiver.is_some()`),
// so they safely reuse the exact same apparatus below (`seed_apparatus`,
// `__ply_seed_corpus` and friends) with no risk of two definitions of the
// same name landing in one generated `#[test] fn` body.

/// A parameter shape `RustType` classifies `Unsupported` -- deliberately,
/// since `RustType::String` is never nested (see its own doc) -- but whose
/// *text* Ply's existing corpus/mutate/trickle apparatus can grow once an
/// `examples:` entry supplies a starting value. Named for what wraps the
/// text, since the wrapping is all this module builds on top of the same
/// seeded `String` strategy: `Option::of` for `OptionString`, a bounded
/// `proptest::collection::vec` for `VecString` -- both proptest combinators
/// that vary structure (present/absent, how many) around a strategy that
/// already varies the text itself, so no new mutation logic is needed for
/// either shape.
///
/// Not attempted this session, and disclosed rather than silently skipped
/// (see TODO.md): `Result<String, E>`, and nested `NonZero`/`Duration`/
/// `f32`/`f64` inside any of these wrappers -- each is a real widening (a
/// `Result`'s `Err` arm needs its own construction story; a number has no
/// existing mutation apparatus the way text does), not a trivial extension
/// of what is here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedableWrap {
    OptionString,
    VecString,
}

/// Classifies an `Unsupported` type's own normalised source text (exactly
/// what [`harness::RustType::Unsupported`] stores, so this reads the same
/// spelling `rust_type_from_syn_at` already produced) as one of the two
/// shapes above, or `None` for anything else -- an opaque struct, a type
/// this session did not open, or a type that is not actually `Unsupported`
/// at all (already buildable, so this mechanism has no job here). `None` is
/// also the honest answer for a shape whose *inner* text cannot be varied
/// -- there is no case in the list above where that is true, but a caller
/// must never assume "classified" implies "always mutable" for some future
/// addition without re-checking this doc.
///
/// Deliberately only the *owned* spellings (`Option<String>`/`Vec<String>`),
/// never `Option<&str>`/`Vec<&str>` -- the wrapped strategy this module
/// builds always produces an owned `String` inside (`__PlySeedStrategy`'s
/// own `Value` type), and splicing that into a call site expecting `&str`
/// needs a borrow this codegen does not add (unlike a bare `&str` top-level
/// parameter, whose existing by-reference call-site handling is a different
/// code path this shape does not go through). Recognising the borrowed
/// spelling here without also fixing the call site would silently generate
/// a harness that fails to compile (`error[E0308]`) -- narrower, but tested,
/// beats broader and unverified.
pub fn classify_seedable_wrap(source: &str) -> Option<SeedableWrap> {
    match source {
        "Option<String>" => Some(SeedableWrap::OptionString),
        "Vec<String>" => Some(SeedableWrap::VecString),
        _ => None,
    }
}

/// What a seeded *plain parameter* needs: which one (by index, so the
/// override lands in the right tuple slot, and by name, for the diagnostic
/// and the runtime stats marker), which shape it wraps, and the seeds
/// pulled from `examples:` at codegen time -- there is no "source 2" here
/// (contrast [`ReceiverSeedPlan`]): nothing rejects an `Option<String>`/
/// `Vec<String>` value the way a fallible constructor rejects text, so
/// there is no runtime "accepted" event to grow the corpus from; it stays
/// exactly the size `examples:` gave it, mutated and trickled from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSeedPlan {
    param_index: usize,
    param_name: String,
    shape: SeedableWrap,
    examples_seeds: Vec<String>,
}

/// Whether `cf`'s own parameters should be seeded at all, and if so, from
/// what. Deliberately narrow, the same way [`plan_receiver_seeding`] is:
///
/// - never for a receiver method (`cf.receiver.is_some()`) -- that shape
///   already has its own seeding story above, and the two apparatuses reuse
///   the same generated variable names, so keeping them mutually exclusive
///   by construction is what makes that reuse safe rather than a latent
///   name collision;
/// - never when more than one of `cf`'s own parameters is otherwise
///   unbuildable -- combining two independent seed pools in one harness is
///   a real widening this session does not attempt, so a fn with two such
///   parameters simply stays refused, honestly, rather than seeding one and
///   silently ignoring the other;
/// - never for a shape [`classify_seedable_wrap`] does not recognise (an
///   opaque type) -- there is no text to grow a corpus from, so seeding it
///   would report cases that never happened;
/// - never with zero seeds -- no `examples:` entry naming a value for this
///   exact parameter means there is nothing to grow from yet, so this stays
///   `None` and the parameter stays refused, exactly like an ungated
///   receiver constructor stays unseeded.
///
/// `None` means every line downstream of this stays exactly as unbuildable
/// as it always was.
pub fn plan_param_seeding(cf: &ContractFn, examples_pool: &[String]) -> Option<ParamSeedPlan> {
    if cf.receiver.is_some() {
        return None;
    }
    let mut bad = cf
        .params
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.ty.is_fuzz_supported());
    let (idx, p) = bad.next()?;
    if bad.next().is_some() {
        return None;
    }
    let RustType::Unsupported(src) = &p.ty else {
        return None;
    };
    let shape = classify_seedable_wrap(src)?;
    let examples_seeds = extract_examples_seed_strings_for_param(examples_pool, &cf.path, idx);
    if examples_seeds.is_empty() {
        return None;
    }
    Some(ParamSeedPlan {
        param_index: idx,
        param_name: p.name.clone(),
        shape,
        examples_seeds,
    })
}

/// Whether `checks` will make *some* declared check actually read
/// `examples` for `cf` -- either directly (`test` compiles every entry
/// into a real assertion) or indirectly (`fuzz` grows its corpus from them,
/// which only ever engages [`plan_param_seeding`] or the receiver's own
/// constructor-seeding, and only when the shape they seed is otherwise
/// unbuildable -- see each one's own doc). `false` is the exact condition
/// `ply-cli`'s `examples_not_run` warning fires on: nothing declared will
/// ever compile or consume these examples, so a false one (or an edit to a
/// true one) changes nothing about the verdict while still being read and
/// fingerprinted as though it mattered (§5.2a).
///
/// Exists so that warning can ask this one question through the same
/// machinery that actually decides it, rather than re-deriving "will this
/// engage seeding" a second time from `cf`'s shape and getting it wrong for
/// exactly the fixtures (`paramseeded`, `textseeded`) built to prove
/// seeding real.
pub fn examples_are_consumed(cf: &ContractFn, checks: &[Check], examples: &[String]) -> bool {
    if checks.iter().any(|c| matches!(c, Check::Test)) {
        return true;
    }
    if !checks.iter().any(|c| matches!(c, Check::Fuzz(_))) {
        return false;
    }
    if plan_param_seeding(cf, examples).is_some() {
        return true;
    }
    if let Some(plan) = &cf.receiver {
        return plan_receiver_seeding(plan, examples).is_some();
    }
    false
}

/// Source 1 for a *plain parameter*'s own corpus, widened past
/// [`extract_examples_seed_strings`]'s constructor-call shape: every string
/// literal found *anywhere inside* the argument written at `param_index` of
/// a call to `fn_path` (matched the same [`harness::last_two_segments`] way
/// every other extractor here matches calls), found anywhere in `examples`.
/// Deliberately structure-agnostic -- `Some("hi".to_string())`, a bare
/// `vec!["a".into(), "b".into()]`, whatever wrapper the parameter's own
/// declared type uses, every string literal inside that one argument's
/// subtree becomes a seed. The same "purely syntactic, zero new vocabulary"
/// contract as the constructor extractor: an unparseable example, or one
/// that never calls `fn_path` at all, contributes nothing here.
pub fn extract_examples_seed_strings_for_param(
    examples: &[String],
    fn_path: &str,
    param_index: usize,
) -> Vec<String> {
    // A plain `syn::visit::Visit` walk (what `extract_examples_seed_strings`
    // above uses) never looks inside a macro invocation's own tokens --
    // `vec!["a".into(), "b".into()]` parses as one opaque `Expr::Macro`, and
    // syn does not know `vec!`'s own grammar to descend into it. Since
    // `Vec<String>`'s own examples are naturally written exactly that way,
    // this scans the argument's raw token stream instead: every literal
    // token, at any nesting depth (including inside a macro's `(...)`/
    // `[...]`/`{...}` group), that parses as a string literal is a seed --
    // structure-agnostic by construction, never needing a dedicated
    // `visit_expr_*` arm for each new wrapper shape this module learns.
    fn collect_string_literals(tokens: proc_macro2::TokenStream, out: &mut Vec<String>) {
        for tt in tokens {
            match tt {
                proc_macro2::TokenTree::Literal(lit) => {
                    if let Ok(syn::Lit::Str(s)) = syn::parse_str::<syn::Lit>(&lit.to_string()) {
                        out.push(s.value());
                    }
                }
                proc_macro2::TokenTree::Group(g) => collect_string_literals(g.stream(), out),
                _ => {}
            }
        }
    }
    struct CallFinder<'a> {
        target: &'a str,
        param_index: usize,
        out: Vec<String>,
    }
    impl<'a> Visit<'a> for CallFinder<'a> {
        fn visit_expr_call(&mut self, node: &'a syn::ExprCall) {
            let func_text = node.func.to_token_stream().to_string().replace(' ', "");
            if harness::last_two_segments(&func_text) == self.target
                && let Some(arg) = node.args.iter().nth(self.param_index)
            {
                collect_string_literals(arg.to_token_stream(), &mut self.out);
            }
            syn::visit::visit_expr_call(self, node);
        }
    }
    let target = harness::last_two_segments(fn_path);
    let mut out = Vec::new();
    for example in examples {
        let Ok(expr) = syn::parse_str::<Expr>(example) else {
            continue;
        };
        let mut finder = CallFinder {
            target: &target,
            param_index,
            out: Vec::new(),
        };
        finder.visit_expr(&expr);
        out.append(&mut finder.out);
    }
    out
}

/// The wrapped strategy expression for one seeded plain parameter, spliced
/// into the checked call's own strategy tuple in place of the `Unsupported`
/// type's usual (nonexistent) one. Reuses [`seed_apparatus`]'s own
/// `__PlySeedStrategy`/`__ply_seed_corpus` verbatim -- always safe, since
/// [`plan_param_seeding`] never returns `Some` when `cf.receiver.is_some()`,
/// the only other place that apparatus is spliced into a harness.
/// Structural variety (present/absent, how many) comes from proptest's own
/// combinators, not a new mutation function: `Option::of` sometimes draws
/// `None` with no text at all, and `collection::vec` independently varies
/// both each element's text and how many elements there are -- exactly the
/// "elements or length" the design brief names for `Vec`.
fn param_seed_wrapped_strategy_expr(plan: &ParamSeedPlan) -> String {
    let inner = "__PlySeedStrategy { corpus: __ply_seed_corpus.clone() }";
    match plan.shape {
        SeedableWrap::OptionString => format!("proptest::option::of({inner})"),
        SeedableWrap::VecString => format!("proptest::collection::vec({inner}, 0..=8)"),
    }
}

/// [`combined_strategy_expr_for`], for the one case it cannot handle: a
/// parameter whose type is `Unsupported` in the ordinary type system, so
/// `plan_for_param`'s default arm would bail building its strategy before
/// ever reaching an override. The seeded parameter's slot is built directly
/// from [`param_seed_wrapped_strategy_expr`]; every other parameter goes
/// through the ordinary `plan_for_param`, unaffected.
fn combined_strategy_expr_for_param_seed(params: &[Param], plan: &ParamSeedPlan) -> Result<String> {
    let mut strategies = Vec::with_capacity(params.len());
    for (i, p) in params.iter().enumerate() {
        if i == plan.param_index {
            strategies.push(param_seed_wrapped_strategy_expr(plan));
        } else {
            strategies.push(plan_for_param(p)?.strategy);
        }
    }
    Ok(match strategies.len() {
        0 => "proptest::strategy::Just(())".to_string(),
        1 => strategies[0].clone(),
        _ => format!("({})", strategies.join(", ")),
    })
}

/// [`params_preamble`]'s counterpart for the same case: the seeded
/// parameter needs no preamble at all (proptest draws the wrapped value
/// directly -- there is no constructor call to splice in, unlike a
/// struct/enum parameter), so its slot is skipped rather than handed to
/// `plan_for_param`, which would bail on it.
fn params_preamble_for_param_seed(params: &[Param], plan: &ParamSeedPlan) -> Result<String> {
    let mut out = String::new();
    for (i, p) in params.iter().enumerate() {
        if i == plan.param_index {
            continue;
        }
        out.push_str(&plan_for_param(p)?.preamble);
    }
    Ok(out)
}

/// [`value_pattern_for`]'s counterpart for the same case: the seeded
/// parameter's pattern is always just its own bare name (an `Unsupported`
/// type is never [`RustType::UserTypeCtor`]/[`RustType::UserTypeFields`],
/// the only shapes whose pattern is not just the parameter's name), so it
/// is bound directly rather than handed to `plan_for_param`, which would
/// bail on it just to compute the same bare name.
fn value_pattern_for_with_param_seed(params: &[Param], plan: &ParamSeedPlan) -> Result<String> {
    let mut names = Vec::with_capacity(params.len());
    for (i, p) in params.iter().enumerate() {
        if i == plan.param_index {
            names.push(p.name.clone());
        } else {
            names.push(plan_for_param(p)?.pattern);
        }
    }
    Ok(match names.len() {
        0 => "_".to_string(),
        1 => names[0].clone(),
        _ => format!("({})", names.join(", ")),
    })
}

/// The generated harness's own runtime support for one seeded `String`
/// parameter, emitted once as nested items inside the generated `#[test]
/// fn` itself -- never a shared dependency, since the harness crate depends
/// on nothing but the target crate and proptest, and this keeps it that
/// way. Builds: the corpus (`Rc<RefCell<Vec<String>>>`, seeded with
/// `examples_seeds` and grown at runtime by `receiver_preamble`'s own
/// push), a counter for how many joined it at runtime, a mutation function
/// (character edit, splice, truncation, repetition, or a verbatim replay --
/// the brief's own list), a uniform-text function reusing the same
/// content/length decisions [`strategy_expr`]'s own `RustType::String` arm
/// already makes, and the `proptest::strategy::Strategy` implementation
/// that draws [`SEED_MUTATE_WEIGHT`]:[`SEED_TRICKLE_WEIGHT`] between them
/// (always trickling when the corpus is still empty, since there is
/// nothing yet to mutate).
fn seed_apparatus(examples_seeds: &[String]) -> String {
    let literal_seeds = examples_seeds
        .iter()
        .map(|s| format!("{}.to_string()", proc_macro2::Literal::string(s)))
        .collect::<Vec<_>>()
        .join(", ");
    let total_weight = SEED_MUTATE_WEIGHT + SEED_TRICKLE_WEIGHT;
    format!(
        "            let __ply_seed_corpus: std::rc::Rc<std::cell::RefCell<Vec<String>>> = \
         std::rc::Rc::new(std::cell::RefCell::new(vec![{literal_seeds}]));\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_seed_corpus_grown = std::cell::Cell::new(0u32);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20fn __ply_seed_uniform(__ply_rng: &mut proptest::test_runner::TestRng) -> String {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20use proptest::prelude::Rng;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_len = __ply_rng.random_range(0u32..={STRING_MAX_CHARS}u32);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20(0..__ply_len).map(|_| {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if __ply_rng.random_range(0u32..10u32) < 9 {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20char::from_u32(__ply_rng.random_range(0x20u32..=0x7eu32)).unwrap_or('a')\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}} else {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20char::from_u32(__ply_rng.random_range(0xa0u32..=0x10ffffu32)).unwrap_or('a')\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}).collect::<String>()\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20fn __ply_seed_mutate(__ply_base: &str, __ply_rng: &mut proptest::test_runner::TestRng) -> String {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20use proptest::prelude::Rng;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let mut __ply_chars: Vec<char> = __ply_base.chars().collect();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20match __ply_rng.random_range(0u32..5u32) {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x200 => {{}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x201 => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if !__ply_chars.is_empty() {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_i = __ply_rng.random_range(0..__ply_chars.len());\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_chars[__ply_i] = if __ply_rng.random_range(0u32..10u32) < 9 {{ char::from_u32(__ply_rng.random_range(0x20u32..=0x7eu32)).unwrap_or('a') }} else {{ char::from_u32(__ply_rng.random_range(0xa0u32..=0x10ffffu32)).unwrap_or('a') }};\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x202 => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_i = __ply_rng.random_range(0..=__ply_chars.len());\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_c = char::from_u32(__ply_rng.random_range(0x20u32..=0x7eu32)).unwrap_or('a');\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_chars.insert(__ply_i, __ply_c);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x203 => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if !__ply_chars.is_empty() {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_new_len = __ply_rng.random_range(0..__ply_chars.len());\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_chars.truncate(__ply_new_len);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20_ => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if !__ply_chars.is_empty() {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_i = __ply_rng.random_range(0..__ply_chars.len());\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_j = __ply_rng.random_range(__ply_i..__ply_chars.len());\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_seg: Vec<char> = __ply_chars[__ply_i..=__ply_j].to_vec();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_at = __ply_rng.random_range(0..=__ply_chars.len());\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20for (__ply_k, __ply_c) in __ply_seg.into_iter().enumerate() {{ __ply_chars.insert(__ply_at + __ply_k, __ply_c); }}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_chars.into_iter().collect()\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20struct __PlySeedValueTree {{ value: String }}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20impl proptest::strategy::ValueTree for __PlySeedValueTree {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20type Value = String;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20fn current(&self) -> String {{ self.value.clone() }}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20fn simplify(&mut self) -> bool {{ false }}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20fn complicate(&mut self) -> bool {{ false }}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20#[derive(Clone)]\n\
         \x20\x20\x20\x20\x20\x20\x20\x20struct __PlySeedStrategy {{ corpus: std::rc::Rc<std::cell::RefCell<Vec<String>>> }}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20impl std::fmt::Debug for __PlySeedStrategy {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ write!(f, \"__PlySeedStrategy\") }}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20impl proptest::strategy::Strategy for __PlySeedStrategy {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20type Tree = __PlySeedValueTree;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20type Value = String;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20fn new_tree(&self, __ply_runner: &mut proptest::test_runner::TestRunner) -> proptest::strategy::NewTree<Self> {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20use proptest::prelude::Rng;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_len = self.corpus.borrow().len();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_value = if __ply_len == 0 || __ply_runner.rng().random_range(0u32..{total_weight}u32) >= {SEED_MUTATE_WEIGHT}u32 {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_seed_uniform(__ply_runner.rng())\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}} else {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_idx = __ply_runner.rng().random_range(0..__ply_len);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_base = self.corpus.borrow()[__ply_idx].clone();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_seed_mutate(&__ply_base, __ply_runner.rng())\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}};\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(__PlySeedValueTree {{ value: __ply_value }})\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}}\n"
    )
}

/// Generates the `ply_fuzz_{fn}` proptest-driven test: `cases` runs of the
/// combined strategy, `requires` as a rejection filter (§5.4c), the
/// `ensures` clause checked in `catch_unwind` (never crashing the whole
/// fuzz loop on an incidental arithmetic panic, same discipline as
/// `contract_rt::render_cex_test`), and the `PLY_FUZZED_CEX` marker printed
/// on the first *shrunk* failing case (proptest's own `TestRunner::run`
/// shrinks before returning `TestError::Fail`, so nothing here re-implements
/// shrinking).
///
/// Returns just the `#[test] fn ply_fuzz_{fn}() { ... }` item text -- the
/// caller assembles it into the per-fn `mod {fn}_harness { ... }` alongside
/// the example/direct-case tests. Never seeded (see
/// [`generate_fuzz_test_with_examples`] for that): every existing caller of
/// this exact function keeps the exact behaviour it always had.
/// The three pieces of generated code the degenerate-route guard needs,
/// alongside the ordinary construction `fuzz_gen` already emits for any
/// `RustType::UserTypeCtor` parameter -- see [`route_distinct_tracking`]'s
/// own doc for how they are built. Every field is an empty string for a fn
/// with no route-built top-level parameter, which is the vast majority: the
/// generated harness is then byte-identical to before this guard existed.
struct RouteDistinctTracking {
    /// Declares one running set per debug-derivable route-built parameter,
    /// spliced in *before* the runner closure so it survives across every
    /// case rather than being rebuilt (and emptied) each time.
    decl: String,
    /// Records this case's own built value into that set -- spliced in
    /// *inside* the closure, right after the parameter is built and before
    /// anything (the `requires` filter included) can reject the case: the
    /// guard counts values Ply actually *built*, not values that went on to
    /// be accepted.
    capture: String,
    /// Prints the split back to `verify`, once, after the runner finishes --
    /// unconditionally, whether the count turned out degenerate or not
    /// (CLAUDE.md's own trap, named for `PLY_FUZZ_OR_SPLIT`: "a threshold
    /// that silently blesses ... print the split always").
    marker: String,
}

/// The degenerate-route guard's own codegen (TODO.md, "the guard this
/// cannot ship without"): a route is a function an author wrote, and
/// nothing stops it from ignoring its own inputs and returning the same
/// value every time -- the one failure a stale-route compile error cannot
/// catch, because the code compiles and runs fine. So every top-level
/// parameter built through a declared route (`RustType::UserTypeCtor` whose
/// `ReceiverPlan::route` is `Some`) gets a running count of how many
/// *distinct* values this run actually built, disclosed unconditionally --
/// "64 cases ran, but only 1 distinct value reached the function" is exactly
/// the sentence this exists to make possible.
///
/// **Deliberately narrow, honestly so**: only ever a *top-level* parameter
/// (`cf.params`, not a route-built value nested inside a `Vec`/`Option`/etc.
/// via composition) -- counting a container's own elements would need the
/// same per-case bookkeeping one level deeper, inside `construct_from_raw_
/// expr`'s own recursion, which has no closure-scoped state to write into
/// (TODO.md carries this as an open item, not a silent gap).
///
/// **The printability condition, not assumed:** counting distinct values
/// needs to tell two built values apart, and the only thing every Rust type
/// offers for free from outside its own crate is nothing at all -- there is
/// no blanket `PartialEq`/`Hash` Ply can rely on. `Debug` text is what this
/// uses instead (a real, if imperfect, proxy: two values whose `Debug`
/// output differs are certainly distinct, though this project makes no
/// claim about the reverse), and only when the type actually derives it
/// (`RouteOrigin::debug_derivable`) -- a type that does not gets the plain,
/// honest disclosure that Ply could not count at all, never an invented
/// number.
fn route_distinct_tracking(cf: &ContractFn) -> RouteDistinctTracking {
    let mut decl = String::new();
    let mut capture = String::new();
    let mut marker = String::new();
    let label = &cf.path;
    for p in &cf.params {
        let RustType::UserTypeCtor(plan) = &p.ty else {
            continue;
        };
        let Some(route) = &plan.route else { continue };
        let name = &p.name;
        let declared_as = &route.declared_as;
        if route.debug_derivable {
            decl.push_str(&format!(
                "        let __ply_route_seen_{name} = std::cell::RefCell::new(\
                 std::collections::BTreeSet::<String>::new());\n"
            ));
            capture.push_str(&format!(
                "            __ply_route_seen_{name}.borrow_mut().insert(format!(\"{{:?}}\", \
                 {name}));\n"
            ));
            marker.push_str(&format!(
                "        eprintln!(\"PLY_ROUTE_DISTINCT|{label}|{name}|{declared_as}|{{}}|{{}}\", \
                 __ply_route_seen_{name}.borrow().len(), __ply_total.get());\n"
            ));
        } else {
            marker.push_str(&format!(
                "        eprintln!(\"PLY_ROUTE_UNPRINTABLE|{label}|{name}|{declared_as}|{{}}\", \
                 __ply_total.get());\n"
            ));
        }
    }
    RouteDistinctTracking {
        decl,
        capture,
        marker,
    }
}

pub fn generate_fuzz_test(cf: &ContractFn, cases: u32, seed: &[u8; 32]) -> Result<String> {
    generate_fuzz_test_with_examples(cf, cases, seed, &[])
}

/// Exactly [`generate_fuzz_test`], plus `examples_pool` -- every `examples:`
/// entry declared anywhere in the crate being verified (not just this fn's
/// own), so a seed written against the constructor from a sibling claim
/// still counts. `examples_pool` is otherwise inert: it only ever matters
/// when `cf` is a receiver method whose constructor [`plan_receiver_seeding`]
/// decides to seed, which is exactly when [`generate_fuzz_test`]'s `&[]`
/// above would also decide *not* to seed for lack of any pool at all -- so
/// passing `&[]` here is indistinguishable from calling
/// `generate_fuzz_test` directly, which is what the plain wrapper does.
pub fn generate_fuzz_test_with_examples(
    cf: &ContractFn,
    cases: u32,
    seed: &[u8; 32],
    examples_pool: &[String],
) -> Result<String> {
    let Some((_closure, _)) = &cf.ensures else {
        bail!(
            "fuzz check requires an #[ply::ensures] clause on `{}` to check against",
            cf.name
        );
    };
    // Widening past the receiver-constructor case (2026-09-01): computed
    // once, up front, exactly like `seed_plan` below computes the receiver
    // case -- `None` for every fn that is not a plain function with exactly
    // one otherwise-unbuildable, example-seeded `Option<String>`/
    // `Vec<String>` parameter, which is the vast majority. When `Some`, this
    // parameter's own type stays `Unsupported` in the ordinary sense
    // (`is_fuzz_supported()` below still says `false` for it), so every
    // check that follows must ask for this plan explicitly rather than
    // re-deriving "is this really unsupported" from the type alone.
    let param_seed_plan = plan_param_seeding(cf, examples_pool);
    if !cf.is_fuzz_supported() && param_seed_plan.is_none() {
        let bad: Vec<String> = cf
            .params
            .iter()
            .filter(|p| !p.ty.is_fuzz_supported())
            .map(|p| format!("{}: {:?}", p.name, p.ty))
            .collect();
        bail!(
            "V0505: `{}` has parameter(s) the fuzz codegen cannot build inputs for: {}",
            cf.name,
            bad.join(", ")
        );
    }
    // Refused by name rather than handed to codegen (defense in depth --
    // `run_fn_checks` in ply-cli checks this first and never reaches here
    // for this shape; this bail exists so a caller that skipped that check
    // gets a loud error instead of a harness that cannot compile).
    if let Some(p) = moved_param_read_in_ensures(cf) {
        bail!(
            "V0506: `{}`'s postcondition reads `{}` after `{}` has already been moved into the \
             call -- `{}: {}` is passed by value, so it no longer exists once `{}` returns. Wrap \
             the read in `old({})` to capture its value before the call, or take `{}` by \
             reference (`&{}`) if `{}` only needs to read it.",
            cf.name,
            p.name,
            p.name,
            p.name,
            p.ty.display_name(),
            cf.name,
            p.name,
            p.name,
            p.ty.display_name(),
            p.name
        );
    }

    // A plain parameter seeded per `param_seed_plan` above needs its own
    // pattern/strategy built through the seeded-aware variants (the
    // ordinary ones would bail trying to build a strategy for its
    // `Unsupported` type) -- `None` for every fn `plan_param_seeding`
    // refused, which takes the exact `value_pattern`/`combined_strategy_
    // expr` calls this always used.
    let target_pattern = match &param_seed_plan {
        Some(plan) => value_pattern_for_with_param_seed(&cf.params, plan)?,
        None => value_pattern(cf),
    };
    let target_strategy = match &param_seed_plan {
        Some(plan) => combined_strategy_expr_for_param_seed(&cf.params, plan)?,
        None => combined_strategy_expr(cf)?,
    };
    let seed_literal = format!(
        "[{}]",
        seed.iter()
            .map(|b| format!("{b}u8"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let seed_hex = seed_hex(seed);
    // Receiver construction (docs/review-self-construction.md's "fourth
    // option", 2026-08-27): a method whose `ContractFn::receiver` is `Some`
    // gets a wider outer strategy/pattern (constructor slot, bounded
    // sequence slot, then the checked call's own) and a preamble that builds
    // the receiver and drives the sequence before the checked call runs.
    // Every other fn's generated harness is byte-identical to before this
    // task -- `receiver` is `None` everywhere else.
    // Seeded generation (docs/reach-measurement-2.md): computed once, up
    // front, from the receiver's own constructor plan and the crate-wide
    // examples pool -- `None` for every receiver whose constructor is not
    // gated, and for every non-receiver fn, which is the vast majority and
    // takes byte-identical paths below to before this feature existed.
    let seed_plan = cf
        .receiver
        .as_ref()
        .and_then(|plan| plan_receiver_seeding(plan, examples_pool));
    let (pattern, strategy, receiver_preamble_text) = match &cf.receiver {
        Some(plan) => {
            let (p, s) = receiver_pattern_and_strategy(
                plan,
                &target_pattern,
                &target_strategy,
                seed_plan.as_ref(),
            )?;
            (
                p,
                s,
                receiver_preamble(cf, plan, &target_pattern, seed_plan.as_ref())?,
            )
        }
        None => (
            target_pattern.clone(),
            target_strategy.clone(),
            String::new(),
        ),
    };
    // Struct/enum parameters (2026-08-27): a plain (non-receiver) function's
    // own parameter may itself be one Ply builds via a constructor or
    // direct field/variant construction -- this is empty for every fn with
    // no such parameter (`params_preamble`'s own doc), so it changes
    // nothing for the vast majority of generated harnesses. For a receiver
    // method, `cf.params` here is the *checked method's own* arguments,
    // deliberately never enriched into a user type (`scan_impls_for_
    // receiver`'s own comment on `target_params`), so this is always empty
    // in that case too -- the receiver's own construction, including its
    // constructor's arguments, is entirely `receiver_preamble_text`'s job.
    let params_preamble_text = match &param_seed_plan {
        Some(plan) => params_preamble_for_param_seed(&cf.params, plan)?,
        None => params_preamble(&cf.params)?,
    };
    let target_args = call_args(cf).join(", ");
    let args = match &cf.receiver {
        Some(_) if target_args.is_empty() => "&__ply_receiver".to_string(),
        Some(_) => format!("&__ply_receiver, {target_args}"),
        None => target_args,
    };
    // Two spellings, deliberately. `fname` is the expression generated
    // code calls -- the bare identifier for a free function, or
    // `Type::method` for a method (`ContractFn::call_expr`, added
    // 2026-08-27: a bare `cf.name` here tried to call a method as if it
    // were an imported free function, which does not compile -- see
    // `wrap_fn_harness_module`'s matching fix to what gets imported).
    // `label` is where the function lives, which is what a reader of the
    // output needs to see. They differ only for a function inside a
    // module, or a method.
    let fname = cf.call_expr();
    let label = &cf.path;
    let ident = cf.ident();

    let requires_check = match &cf.requires {
        Some((expr, _)) => {
            let cond = expr.to_token_stream().to_string();
            format!(
                "if !({cond}) {{ __ply_rejected.set(__ply_rejected.get() + 1); \
                 return Err(proptest::test_runner::TestCaseError::reject(\"requires filter\")); }}\n            "
            )
        }
        None => String::new(),
    };

    // A receiver method's postcondition is spliced into this generated test
    // as a free-standing expression, outside the `impl` block it was
    // written in -- so a bare `self` in it (the most natural thing a
    // method's own promise says: relating its result to the receiver it was
    // called on) is rewritten to `__ply_receiver`, the binding
    // `receiver_preamble` above already built the receiver under. A no-op
    // for every non-receiver fn (`cf.receiver` is `None`), so nothing
    // changes for the vast majority of generated harnesses. Done *before*
    // `old()` is lifted, so `old(self.a)` still reads the receiver's value
    // on entry rather than the (post-call) `__ply_receiver` reference.
    let ensures_body = &cf.ensures.as_ref().unwrap().0.body;
    let self_rewritten_body = if cf.receiver.is_some() {
        crate::contract_rt::rewrite_self_to_receiver(ensures_body)
    } else {
        (**ensures_body).clone()
    };

    // `old(expr)` -- the value `expr` had on entry -- is read into a
    // binding of its own before the call, which is the only way a harness
    // built out of ordinary Rust can honour it (§5.4a).
    let (checked_body, entry_values) = crate::contract_rt::lift_entry_values(&self_rewritten_body);
    let entry_lets = crate::contract_rt::entry_value_lets(&entry_values, &" ".repeat(12));
    let widened = crate::contract_rt::widen(&checked_body, cf).to_string();

    // The branch-decided measurement (CLAUDE.md, 2026-09-02): "record which
    // branch of the promise actually decided each case". A promise whose
    // top level is `||` is checked left to right, exactly like the `||` it
    // is -- but nothing recorded *which* side was the one that turned out
    // true, so a promise satisfied almost entirely by its first side (the
    // real defect found pointing Ply at `semver`'s `Version::parse`, whose
    // promise is `!text.contains(' ') || result.is_err()`) reported the
    // same unqualified `fuzzed(n)` as one genuinely exercising every side.
    // `flatten_top_level_or` returns `None` for any shape that is not a
    // bare `||` at the top -- this is where that refusal takes effect:
    // `or_arms` stays `None`, `check_expr` falls back to the plain
    // `widened` boolean exactly as before, and nothing below this point
    // changes for the vast majority of fns, which have no top-level `||`
    // at all.
    let or_arms = crate::contract_rt::flatten_top_level_or(&checked_body);
    let (or_cells_decl, check_expr, or_split_marker) = match &or_arms {
        Some(arms) => {
            let mut cells_decl = String::new();
            for i in 0..arms.len() {
                cells_decl.push_str(&format!(
                    "        let __ply_or_hit_{i} = std::cell::Cell::new(0u32);\n"
                ));
            }
            // Left to right, exactly the way `||` itself reads: each arm's
            // condition is only ever evaluated once every earlier arm has
            // already come back false, the same short-circuit `if`/`else
            // if` already guarantees in ordinary Rust -- so an arm that
            // never runs because an earlier one already decided the case
            // is never evaluated at all here either, never credited and
            // never penalized for what it "would have" said (CLAUDE.md's
            // first trap: "evaluating every branch in order to count it").
            let mut block = String::from("{\n");
            for (i, arm) in arms.iter().enumerate() {
                let widened_arm = crate::contract_rt::widen(arm, cf).to_string();
                let keyword = if i == 0 { "if" } else { "} else if" };
                block.push_str(&format!(
                    "            {keyword} ({widened_arm}) {{ __ply_or_hit_{i}.set(__ply_or_hit_{i}.get() + 1); true\n"
                ));
            }
            block.push_str("            } else { false }\n        }");
            let counts_fmt = vec!["{}"; arms.len()].join(",");
            let counts_args = (0..arms.len())
                .map(|i| format!("__ply_or_hit_{i}.get()"))
                .collect::<Vec<_>>()
                .join(", ");
            let marker = format!(
                "        eprintln!(\"PLY_FUZZ_OR_SPLIT|{label}|{counts_fmt}\", {counts_args});\n"
            );
            (cells_decl, block, marker)
        }
        None => (String::new(), widened.clone(), String::new()),
    };

    // The degenerate-route guard (TODO.md, "the guard this cannot ship
    // without"): empty for every fn with no top-level parameter built
    // through §5.4b's declared-route mechanism, which is the vast majority
    // and generates byte-identical code to before this feature existed --
    // see `route_distinct_tracking`'s own doc for what the three pieces do.
    let RouteDistinctTracking {
        decl: route_decl,
        capture: route_capture,
        marker: route_marker,
    } = route_distinct_tracking(cf);

    // Every field's *display text* is computed into its own binding **before**
    // the call, never inline inside the failure-branch marker build that
    // used to reference `p.name` directly there. Found by this task's own
    // fixture (2026-08-27): a by-value, non-`Copy` parameter whose
    // postcondition never reads it back (so `moved_param_read_in_ensures`
    // correctly does not refuse it -- nothing in the *contract* touches a
    // moved value) was still moved into `{fname}({args})`, and the marker
    // build the failure branches splice in *after* that call went right on
    // reading `p.name` again to format it -- `error[E0382]: borrow of moved
    // value`, for every by-value `String`/`Vec`/`BTreeSet` parameter whose
    // contract does not itself read it post-call. Pre-computing every
    // field's text before the move (alongside `old()`'s own entry-value
    // bindings, which solve the identical problem for the *contract* side)
    // closes it for every parameter shape at once, not just `String`'s.
    // Wrapped in `format!("{{}}", ...)`, never spliced as a bare binding
    // initializer: `marker_display_expr`'s default (scalar) arm returns the
    // *bare variable reference* (`"x"`), relying on the surrounding
    // `format!("{}", x)` call in the old inline site to `Display` it rather
    // than producing a `String` itself -- `let _: String = x;` does not
    // compile for a `u32` (`error[E0308]`, the regression this task's own
    // `movedparam` e2e fixture caught: `scalar`, an ordinary `x: u32` fn
    // wholly unrelated to `String`/`Vec`, failed to build). Every other arm
    // already evaluates to a `String` (`format!(...)`, or a block ending in
    // one), so wrapping is a harmless second `Display` pass for those, not
    // a second escaping/formatting step.
    let marker_precompute: String = cf
        .params
        .iter()
        .map(|p| {
            format!(
                "            let __ply_marker_val_{name}: String = format!(\"{{}}\", {expr});\n",
                name = p.name,
                expr = marker_display_expr(&p.ty, &p.name)
            )
        })
        .collect();
    let mut marker_build = String::from("let mut __ply_marker = String::new();\n");
    marker_build.push_str(&format!(
        "            __ply_marker.push_str(\"PLY_FUZZED_CEX|{label}|\");\n"
    ));
    for (i, p) in cf.params.iter().enumerate() {
        if i > 0 {
            marker_build.push_str("            __ply_marker.push(';');\n");
        }
        marker_build.push_str(&format!(
            "            __ply_marker.push_str(&format!(\"{name}={{}}\", __ply_marker_val_{name}));\n",
            name = p.name
        ));
    }

    // Seeded generation's own runtime support and the honest-provenance
    // marker it reports (docs/reach-measurement-2.md): both empty strings
    // for the vast majority of fns (`seed_plan` is `None`), so the
    // generated test is byte-identical to before this feature existed --
    // the honesty condition CLAUDE.md calls out by name ("a seeded verdict
    // must never be indistinguishable from an unseeded one") cuts both
    // ways, and an unseeded run must carry no trace of this mechanism
    // either.
    // The two seeding mechanisms are mutually exclusive by construction
    // (`plan_param_seeding` refuses whenever `cf.receiver.is_some()`, the
    // only other place `seed_plan` is `Some`), so it is safe to build the
    // exact same apparatus from either one -- never both, so never two
    // definitions of the same generated name in one `#[test] fn` body.
    let seed_setup = match (&seed_plan, &param_seed_plan) {
        (Some(sp), _) => seed_apparatus(&sp.examples_seeds),
        (None, Some(psp)) => seed_apparatus(&psp.examples_seeds),
        (None, None) => String::new(),
    };
    // Printed unconditionally (never gated on the outcome below) so
    // `verify` can learn the real provenance -- how many seeds came from
    // `examples:`, how many the constructor accepted at runtime, and the
    // same rejected/total counts the high-rejection warning already
    // computes -- whichever way the run ends: a clean pass, a violation, or
    // proptest abandoning the run entirely for lack of any accepted value
    // to grow from.
    // A plain-parameter-seeded run's own marker additionally names the
    // parameter (`|param={name}`) -- there is no rejection dynamic to
    // report here (nothing gates an `Option<String>`/`Vec<String>` value
    // the way a fallible constructor gates text), so `accepted` always
    // reads 0 (`__ply_seed_corpus_grown` is never incremented for this
    // mechanism -- see `ParamSeedPlan`'s own doc on why there is no
    // "source 2"), which is the honest count: nothing grew beyond what
    // `examples:` gave it. `__ply_rej`/`__ply_tot` still track whatever this
    // fn's own `#[ply::requires]` rejected, if it has one -- unrelated to
    // seeding, and correct either way.
    let seed_stats_marker = match (&seed_plan, &param_seed_plan) {
        (Some(sp), _) => format!(
            "eprintln!(\"PLY_FUZZ_SEED_STATS|{label}|examples={ex}|accepted={{}}|rejected={{}}|total={{}}\", __ply_seed_corpus_grown.get(), __ply_rej, __ply_tot);\n            ",
            ex = sp.examples_seeds.len()
        ),
        (None, Some(psp)) => format!(
            "eprintln!(\"PLY_FUZZ_SEED_STATS|{label}|examples={ex}|accepted={{}}|rejected={{}}|total={{}}|param={pname}\", __ply_seed_corpus_grown.get(), __ply_rej, __ply_tot);\n            ",
            ex = psp.examples_seeds.len(),
            pname = psp.param_name,
        ),
        (None, None) => String::new(),
    };

    Ok(format!(
        "    #[test]\n\
         \x20\x20\x20\x20fn ply_fuzz_{ident}() {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"PLY_FUZZ_SEED|{label}|{seed_hex}\");\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let mut __ply_runner = proptest::test_runner::TestRunner::new_with_rng(\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20proptest::test_runner::Config {{ cases: {cases}, failure_persistence: None, ..proptest::test_runner::Config::default() }},\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20proptest::test_runner::TestRng::from_seed(\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20proptest::test_runner::RngAlgorithm::ChaCha, &{seed_literal}),\n\
         \x20\x20\x20\x20\x20\x20\x20\x20);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_rejected = std::cell::Cell::new(0u32);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_total = std::cell::Cell::new(0u32);\n\
         {or_cells_decl}\
         {route_decl}\
         {seed_setup}\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_strategy = {strategy};\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_outcome = __ply_runner.run(&__ply_strategy, |{pattern}| {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_total.set(__ply_total.get() + 1);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20{params_preamble_text}{route_capture}{requires_check}{receiver_preamble_text}{entry_lets}{marker_precompute}let __ply_call_result = {fname}({args});\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let result = &__ply_call_result;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {check_expr}));\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20match __ply_ok {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(true) => Ok(()),\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(false) => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20{marker_build}\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"{{}}\", __ply_marker);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(proptest::test_runner::TestCaseError::fail(\"ensures violated\"))\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(_) => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20{marker_build}\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"{{}}\", __ply_marker);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(proptest::test_runner::TestCaseError::fail(\"the contract's own check panicked at this input\"))\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}});\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_rej = __ply_rejected.get();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_tot = __ply_total.get();\n\
         {seed_stats_marker}\
         {or_split_marker}\
         {route_marker}\
         \x20\x20\x20\x20\x20\x20\x20\x20match __ply_outcome {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(()) => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if __ply_tot > 0 && (__ply_rej as f64) / (__ply_tot as f64) > 0.5 {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"PLY_FUZZ_HIGH_REJECT|{label}|{{}}/{{}}\", __ply_rej, __ply_tot);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(proptest::test_runner::TestError::Abort(reason)) => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"PLY_FUZZ_ABORT|{label}|{{}}|accepted={{}}|rejected={{}}\", reason, __ply_tot - __ply_rej, __ply_rej);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(e) => panic!(\"proptest found a failing case for `{label}`: {{}}\", e),\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20}}\n",
    ))
}

/// Renders one `ply.yaml` `examples` entry (§5.4a: exempt from the contract
/// subset -- "arbitrary Rust `==` expressions") as a plain `#[test]`.
/// `E0501`-shaped parse failure surfaces as an error, never a silently
/// skipped example.
///
/// Takes the checked function itself, `cf`, rather than a bare name string,
/// and builds the test's name from [`ContractFn::ident`] -- the same safe
/// identifier [`generate_fuzz_test_with_examples`] already builds its own
/// `ply_fuzz_{ident}` test name from (it turns `Type::method` into
/// `Type_method`). This function used to take the checked function's raw
/// `::`-qualified path as a plain `&str` and splice it straight into the
/// generated name, so a method's test read `fn
/// ply_example_Type::method_01()` -- not a legal identifier at all, and the
/// harness crate failed with `error: invalid path separator in function
/// definition`. Every fixture exercising this codegen used a free function
/// (whose path has no `::` to go wrong), so the break was never seen even
/// though nearly everything in a real library is a method (found
/// 2026-09-01, verified by hand against `semver`'s `Version::cmp_precedence`).
/// Taking `cf` and deriving the ident in here, the same way the fuzz-test
/// generator already does, makes passing the wrong string impossible rather
/// than merely documented against -- there is only ever one place that
/// turns a checked function into a safe identifier.
pub fn generate_example_test(cf: &ContractFn, index: u32, example_src: &str) -> Result<String> {
    let expr: Expr = syn::parse_str(example_src).map_err(|e| {
        anyhow::anyhow!(
            "E0501: could not parse `examples` entry `{example_src}` as a Rust expression: {e}"
        )
    })?;
    let text = expr.to_token_stream().to_string();
    // The entry is echoed back into the assert's failure message, so it has
    // to be escaped for a Rust string literal: an entry containing a `"`
    // (`f(0) == "zero"`) otherwise closes the literal early and the harness
    // crate fails to build with a *syntax* error inside Ply's own generated
    // file -- burying the user's real mistake (2026-08-24 M4 review, D1's
    // own probe).
    let escaped_src = example_src.replace('\\', "\\\\").replace('"', "\\\"");
    let ident = cf.ident();
    Ok(format!(
        "    #[test]\n\
         \x20\x20\x20\x20fn ply_example_{ident}_{index:02}() {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20assert!({text}, \"example failed: `{escaped_src}` does not hold\");\n\
         \x20\x20\x20\x20}}\n",
    ))
}

/// A small, fixed boundary literal for `ty` (0/1/MAX for unsigned,
/// 0/MIN/MAX for signed, true/false for bool, a handful of lengths for
/// `Vec`/`BTreeSet`) -- the concrete inputs behind the `test` check's
/// "generated direct contract cases" (§10 M4, §5.4c).
fn boundary_literals(ty: &RustType) -> Vec<String> {
    match ty {
        RustType::U8 | RustType::U16 | RustType::U32 | RustType::U64 | RustType::Usize => {
            let n = ty.scalar_rust_name().unwrap();
            vec![format!("0{n}"), format!("1{n}"), format!("{n}::MAX")]
        }
        RustType::I8 | RustType::I16 | RustType::I32 | RustType::I64 | RustType::Isize => {
            let n = ty.scalar_rust_name().unwrap();
            vec![format!("0{n}"), format!("{n}::MIN"), format!("{n}::MAX")]
        }
        RustType::Bool => vec!["true".to_string(), "false".to_string()],
        RustType::VecU8 => vec![
            "vec![]".to_string(),
            "vec![0u8]".to_string(),
            "vec![1u8; 8]".to_string(),
        ],
        // Composition (2026-09-02) widened `Vec`'s own element gate past a
        // plain scalar -- but a fixed boundary *literal* still only makes
        // sense when the element has one: `scalar_rust_name` returning
        // `None` for the new inner shapes (`String`, a user struct, another
        // container) now means "no literal", never the old `.unwrap_or
        // ("i64")` fallback, which would have silently spliced a wrong-
        // typed literal (`vec![0i64]` for a `Vec<String>`) into generated
        // code and failed to *compile*, not merely to typecheck sensibly.
        RustType::Vec(inner) => match inner.scalar_rust_name() {
            Some(n) => vec![
                "vec![]".to_string(),
                format!("vec![0{n}]"),
                format!("vec![1{n}; 8]"),
            ],
            None => vec![],
        },
        RustType::BTreeSet(inner) => match inner.scalar_rust_name() {
            Some(n) => vec![
                "std::collections::BTreeSet::new()".to_string(),
                format!("std::collections::BTreeSet::from([0{n}])"),
                format!("std::collections::BTreeSet::from([0{n}, 1{n}, 2{n}])"),
            ],
            None => vec![],
        },
        RustType::Char => vec![
            "'a'".to_string(),
            "'0'".to_string(),
            "'\\u{10FFFF}'".to_string(),
        ],
        RustType::Option(inner) => {
            let mut out = vec!["None".to_string()];
            for lit in boundary_literals(inner) {
                out.push(format!("Some({lit})"));
            }
            out
        }
        RustType::Result(ok, err) => {
            let mut out = Vec::new();
            for lit in boundary_literals(ok) {
                out.push(format!("Ok({lit})"));
            }
            for lit in boundary_literals(err) {
                out.push(format!("Err({lit})"));
            }
            out
        }
        RustType::Array(inner, n) => boundary_literals(inner)
            .into_iter()
            .map(|lit| format!("[{lit}; {n}]"))
            .collect(),
        RustType::NonZero(inner) => {
            let Some(suffix) = inner.nonzero_suffix() else {
                return vec![];
            };
            let n = inner.scalar_rust_name().unwrap_or("u32");
            vec![
                format!("std::num::NonZero{suffix}::new(1{n}).unwrap()"),
                format!("std::num::NonZero{suffix}::new({n}::MAX).unwrap()"),
            ]
        }
        RustType::Duration => vec![
            "std::time::Duration::ZERO".to_string(),
            "std::time::Duration::from_nanos(1)".to_string(),
            "std::time::Duration::new(u64::MAX, 999_999_999)".to_string(),
        ],
        // Finite boundaries only, matching the fuzz tier's own NaN/infinity
        // exclusion (`strategy_expr`'s doc comment): zero and the two
        // finite extremes, never `NAN`/`INFINITY`/`NEG_INFINITY`. `{n}::MIN`
        // for a float is the most negative *finite* value (unlike an
        // integer's `MIN`, this is not the sign-flipped complement of
        // `MAX` -- both are finite here, which is exactly the point).
        RustType::F32 | RustType::F64 => {
            let n = ty.scalar_rust_name().unwrap();
            vec![format!("0.0{n}"), format!("{n}::MIN"), format!("{n}::MAX")]
        }
        // The empty string, a single ASCII character, a string right at the
        // sampled length bound, and -- deliberately, not an afterthought --
        // one genuinely multi-byte case (accented Latin, CJK, and an emoji
        // together) so the `test` tier's own concrete cases exercise the
        // exact byte-vs-character bug class the fuzz strategy's content
        // choice targets, not only its randomly sampled cases.
        RustType::String => vec![
            "String::new()".to_string(),
            "\"a\".to_string()".to_string(),
            format!("\"a\".repeat({STRING_MAX_CHARS})"),
            "\"h\\u{e9}llo, \\u{4e16}\\u{754c}! \\u{1f389}\".to_string()".to_string(),
        ],
        // Never reached: return-only shapes, never a parameter's.
        RustType::SelfType | RustType::Unit | RustType::Unsupported(_) => vec![],
        // No fixed boundary literal makes sense for a value built by calling
        // a constructor or filling in fields -- there is no "zero" or "MAX"
        // for `Quota`. An empty list here means `generate_direct_contract_
        // cases` skips the `test` check's direct cases for this fn
        // entirely (its own `literal_sets.iter().any(|s| s.is_empty())`
        // guard), the same honest "nothing generated, not a crash" answer
        // it already gives for a receiver method -- `fuzz(n)` still runs
        // and still earns a real verdict.
        RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => vec![],
        // Same reasoning as `Vec`'s own arm above, reusing it directly: a
        // slice's owned representation *is* a `Vec<T>` (see
        // `RustType::Slice`'s own doc), so a literal that builds one is
        // exactly a `Vec` literal -- the by-ref call site (`&name`) coerces
        // it to `&[T]` regardless of which literal was used to build it.
        RustType::Slice(inner) => boundary_literals(&RustType::Vec(inner.clone())),
        // `Box::new(v)` around every literal `T` already has, when it has
        // any -- free and correct, the same recursive pattern `Option`/
        // `Result`/`Array` above already use.
        RustType::BoxT(inner) => boundary_literals(inner)
            .into_iter()
            .map(|lit| format!("Box::new({lit})"))
            .collect(),
        // No fixed literal makes sense for an arbitrary tuple or map the
        // same general way `Option`/`Result`/`Array` recurse -- doing so
        // correctly for every arity/element combination is real, separate
        // work this task did not take on. `fuzz(n)` still runs and still
        // earns a real verdict; only the `test` tier's own fixed direct
        // cases are narrower here, the same honest degradation the
        // `UserTypeCtor`/`UserTypeFields` arm above already accepts.
        RustType::Tuple(_) | RustType::BTreeMap(_, _) => vec![],
    }
}

/// Generates a small, fixed battery of "direct contract case" tests: real
/// concrete inputs (boundary literals per parameter, diagonally zipped
/// rather than a full cross product, to keep the generated file small) run
/// through the real function with `ensures` asserted directly -- §5.4c's
/// "generated direct contract cases (concrete inputs run through the real
/// function, contract asserted)". `requires`, if present, gates each case
/// (a case whose literals fail `requires` is skipped, not asserted against).
/// Silently produces nothing for a fn the fuzz codegen cannot build inputs
/// for (already reported elsewhere as `unsupported`/`V0505`) or with no
/// `ensures` to assert.
pub fn generate_direct_contract_cases(cf: &ContractFn) -> String {
    let Some((closure, _)) = &cf.ensures else {
        return String::new();
    };
    if !cf.is_fuzz_supported() {
        return String::new();
    }
    // A receiver method has no *concrete* receiver value this tier builds --
    // only the fuzz tier's own randomly-sampled constructor-plus-sequence
    // does (2026-08-27, receiver construction). Producing nothing here is
    // the honest answer, the same one this fn already gives for a shape it
    // cannot build inputs for at all; it is not the same as refusing the fn
    // -- `fuzz(n)` above still runs.
    if cf.receiver.is_some() {
        return String::new();
    }
    if moved_param_read_in_ensures(cf).is_some() {
        return String::new();
    }
    let literal_sets: Vec<Vec<String>> =
        cf.params.iter().map(|p| boundary_literals(&p.ty)).collect();
    if literal_sets.iter().any(|s| s.is_empty()) {
        return String::new();
    }
    // A zero-parameter fn has no literal sets to size cases from at all
    // (`.max()` on an empty iterator is `None`) -- one case, calling it with
    // no arguments, is still a real direct-contract check, not nothing.
    // Found silently generating zero cases (adversarial review, 2026-08-27,
    // alongside the fuzz-tier `Just(())` fix this mirrors).
    let n_cases = if cf.params.is_empty() {
        1
    } else {
        literal_sets.iter().map(|s| s.len()).max().unwrap_or(0)
    };
    let (checked_body, entry_values) = crate::contract_rt::lift_entry_values(&closure.body);
    let entry_lets = crate::contract_rt::entry_value_lets(&entry_values, &" ".repeat(8));
    let widened = crate::contract_rt::widen(&checked_body, cf).to_string();
    let requires_cond = cf
        .requires
        .as_ref()
        .map(|(e, _)| e.to_token_stream().to_string());

    let mut out = String::new();
    for case_idx in 0..n_cases {
        let mut lets = String::new();
        for (p, set) in cf.params.iter().zip(literal_sets.iter()) {
            let lit = &set[case_idx % set.len()];
            lets.push_str(&format!("        let {name} = {lit};\n", name = p.name));
        }
        let args = call_args(cf).join(", ");
        let guard = match &requires_cond {
            Some(cond) => format!("if !({cond}) {{ return; }}\n        "),
            None => String::new(),
        };
        out.push_str(&format!(
            "    #[test]\n\
             \x20\x20\x20\x20fn ply_direct_{ident}_{case_idx:02}() {{\n\
             {lets}\
             \x20\x20\x20\x20\x20\x20\x20\x20{guard}{entry_lets}let __ply_call_result = {fname}({args});\n\
             \x20\x20\x20\x20\x20\x20\x20\x20let result = &__ply_call_result;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20assert!({widened}, \"direct contract case for `{label}` broke its postcondition\");\n\
             \x20\x20\x20\x20}}\n",
            fname = cf.call_expr(),
            label = cf.path,
            ident = cf.ident(),
            entry_lets = entry_lets,
        ));
    }
    out
}

/// Assembles one `#[cfg(test)] mod {fn}_harness { ... }` from the generated
/// bodies (fuzz test, example tests, direct-case tests), importing the
/// target fn from `target_crate_ident` (the target crate's Rust identifier
/// -- its `[lib] name`, not necessarily its package name).
/// Every struct/enum type name Ply itself constructs somewhere in `cf` --
/// a parameter's own type, or (recursively) a constructor argument or a
/// field/variant's own type -- deduplicated, in first-seen order, and never
/// repeating `cf.import_path()` (the `use` `wrap_fn_harness_module` already
/// emits to call the checked function itself).
///
/// Struct/enum parameters (2026-08-27): `call_expr()`'s own import only
/// brings the checked function (or its enclosing type, for a method) into
/// scope, never a *parameter's* type -- `wrap_fn_harness_module` needs one
/// more `use` per such type, or the generated harness fails to compile with
/// "cannot find struct/enum `X` in this scope" the moment it names one in a
/// constructor call or a field/variant literal.
///
/// A real `HashSet` backs the dedup, seeded with `cf.import_path()`, rather
/// than each call site separately checking "have I already emitted this
/// name" (found against `semver`, docs/reach-measurement-2.md, defect 2): a
/// method whose parameter shares its receiver's type -- `same_as(&self,
/// other: &Self)`, the ordinary shape behind `merge`/`cmp`/`min`/`max` too --
/// used to get the receiver's own type name from `import_path()` *and*
/// again from this scan (which only checked against its own output, never
/// against `import_path()`), so the generated harness imported the same
/// type twice: `use Pair;` twice in one module is `error[E0252]: the name
/// `Pair` is defined multiple times`, and the check ran zero cases.
/// Seeding the same set with `import_path()` up front makes "the receiver's
/// type" and "a second parameter of the same type" the same case as the
/// dedup two parameters of one type already needed, instead of a second
/// special case next to it.
/// Resolves one `ReceiverPlan`'s own `import_path` into the exact `use`
/// target codegen writes: an ordinary (in-crate) parameter's `import_path`
/// is a bare name this crate declares, so it needs `target_crate_ident`
/// in front of it -- but a cross-crate route's (`ReceiverPlan::route`'s own
/// `outside_crate`, §5.4b's extension, defect 2) is already a full,
/// absolute path (`std::ffi::OsString`), and prefixing it with the target
/// crate's own name would write a path that does not exist and does not
/// compile.
fn resolved_import(plan: &harness::ReceiverPlan, target_crate_ident: &str) -> String {
    match &plan.route {
        Some(r) if r.outside_crate => plan.import_path.clone(),
        _ => format!("{target_crate_ident}::{}", plan.import_path),
    }
}

/// Every `use` line a value of `ty` needs in the generated harness, walking
/// into nested user types. Shared by the function path and the state
/// invariant path so the two can never disagree about how a type is named.
pub(crate) fn collect_type_imports(
    ty: &RustType,
    target_crate_ident: &str,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    match ty {
        RustType::UserTypeCtor(plan) => {
            let full = resolved_import(plan, target_crate_ident);
            if seen.insert(full.clone()) {
                out.push(full);
            }
            for p in &plan.ctor_params {
                collect_type_imports(&p.ty, target_crate_ident, seen, out);
            }
        }
        RustType::UserTypeFields(plan) => {
            // A field/variant plan (rule 2, direct construction) is
            // never route-built -- `resolve_declared_route` only ever
            // produces `UserTypeCtor` -- so this is always an ordinary,
            // in-crate bare name.
            let full = format!("{target_crate_ident}::{}", plan.import_path);
            if seen.insert(full.clone()) {
                out.push(full);
            }
            match &plan.shape {
                harness::UserTypeShape::Struct(fields) => {
                    for f in fields {
                        collect_type_imports(&f.ty, target_crate_ident, seen, out);
                    }
                }
                harness::UserTypeShape::Enum(variants) => {
                    for (_, fields) in variants {
                        for f in fields {
                            collect_type_imports(&f.ty, target_crate_ident, seen, out);
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
        | RustType::BoxT(inner) => collect_type_imports(inner, target_crate_ident, seen, out),
        RustType::Result(ok, err) => {
            collect_type_imports(ok, target_crate_ident, seen, out);
            collect_type_imports(err, target_crate_ident, seen, out);
        }
        RustType::Tuple(items) => {
            for item in items {
                collect_type_imports(item, target_crate_ident, seen, out);
            }
        }
        RustType::BTreeMap(key, value) => {
            collect_type_imports(key, target_crate_ident, seen, out);
            collect_type_imports(value, target_crate_ident, seen, out);
        }
        _ => {}
    }
}

fn extra_type_imports(cf: &ContractFn, target_crate_ident: &str) -> Vec<String> {
    let walk = collect_type_imports;

    let mut seen = std::collections::HashSet::new();
    seen.insert(format!("{target_crate_ident}::{}", cf.import_path()));
    let mut out = Vec::new();
    for p in &cf.params {
        walk(&p.ty, target_crate_ident, &mut seen, &mut out);
    }
    if let Some(plan) = &cf.receiver {
        for p in &plan.ctor_params {
            walk(&p.ty, target_crate_ident, &mut seen, &mut out);
        }
        // The operations too, not only the constructor (2026-09-04). A
        // receiver is built by calling a constructor and then a sequence of
        // the type's own methods, and an argument to one of *those* is a
        // value the harness constructs exactly like any other -- so a type
        // reached only that way needs its `use` line just as much. Without
        // this the harness wrote a bare type name that resolved nowhere,
        // the harness crate failed to compile, and because one harness is
        // shared by every claim in a crate, every claim in it came back a
        // tool error -- including claims with nothing but scalars in them.
        // Ply's own library was in exactly that state: six promises, none
        // of them ever reported as anything but a broken harness, because
        // one status set is built by inserting an enum declared in a module.
        for op in &plan.operations {
            for p in &op.params {
                walk(&p.ty, target_crate_ident, &mut seen, &mut out);
            }
        }
    }
    out
}

/// Every path expression's leading identifier in `expr`, collected without
/// judging whether it names a type -- `Ordering` from `Ordering::Greater`,
/// `x` from a bare parameter read, `result` from the closure's own
/// parameter, all land in the same set. Over-collection costs nothing:
/// [`contract_referenced_use_imports`] only acts on names that also happen
/// to be a key in the file's own `use` aliases, and a parameter or `result`
/// never is one. Mirrors `find_moved_param_read`'s own `Visit`-based walk
/// two functions above it in this file.
fn collect_leading_idents(expr: &Expr, out: &mut BTreeSet<String>) {
    struct Finder<'a> {
        out: &'a mut BTreeSet<String>,
    }
    impl<'a> Visit<'a> for Finder<'a> {
        fn visit_expr_path(&mut self, node: &'a syn::ExprPath) {
            if let Some(seg) = node.path.segments.first() {
                self.out.insert(seg.ident.to_string());
            }
            syn::visit::visit_expr_path(self, node);
        }
    }
    Finder { out }.visit_expr(expr);
}

/// The counterpart to `extra_type_imports` for a name the *contract text*
/// refers to directly, rather than one reached by walking a parameter or
/// the receiver (docs/reach-measurement-2.md: a postcondition naming
/// `Ordering` -- not a parameter, not the return type, just a name in the
/// `#[ply::ensures]` expression -- failed the generated harness with
/// `error[E0433]: cannot find type Ordering in this scope`; nothing in
/// `extra_type_imports` walks contract text at all).
///
/// Resolved only through `cf.use_aliases` -- exactly the `use` items the
/// file the checked fn lives in already declared, matched against every
/// name `collect_leading_idents` finds in `requires`/`ensures`. **Not** a
/// blanket re-emission of every `use` in that file, and **not** a glob
/// import of the target crate's own root: both were considered and
/// rejected. A glob import of the target crate cannot reach this defect's
/// own reproduction at all -- `Ordering` is `std::cmp::Ordering`, not
/// anything the target crate exports, so only re-emitting the file's own
/// `use std::cmp::Ordering;` (or an equivalent path) ever brings it into
/// scope. And blindly re-emitting *every* `use` item in the file -- rather
/// than only the ones the contract actually names -- risks a name that is
/// private to the target crate for a completely unrelated reason (some
/// helper the checked fn's contract never mentions): re-emitting that from
/// what is, for the fuzz engine, a separate downstream crate would newly
/// fail to compile on a function whose contract asked for nothing new, the
/// exact "one fix breaks a neighbour" shape this project treats as
/// seriously as a missing one. Scoping to referenced names keeps a failure
/// possible only where the contract itself names something the crate does
/// not export -- a real, actionable compiler error, not silent collateral
/// damage.
///
/// `crate::`-prefixed segments are rewritten to `target_crate_ident` (an
/// external harness crate has no `crate::` of its own that could mean the
/// target); a `self::`/`super::`-prefixed path is skipped outright --
/// resolving those needs the declaring module's own position in the crate
/// tree, which this scan does not carry, the same crate-root assumption
/// `extra_type_imports`'s own doc already states for a bare struct/enum
/// name.
fn contract_referenced_use_imports(cf: &ContractFn, target_crate_ident: &str) -> Vec<String> {
    let mut idents = BTreeSet::new();
    if let Some((expr, _)) = &cf.requires {
        collect_leading_idents(expr, &mut idents);
    }
    if let Some((closure, _)) = &cf.ensures {
        collect_leading_idents(&closure.body, &mut idents);
    }
    let mut out = Vec::new();
    for ident in idents {
        let Some(segments) = cf.use_aliases.get(&ident) else {
            continue;
        };
        match segments.first().map(String::as_str) {
            Some("self") | Some("super") => continue,
            Some("crate") => {
                let mut rewritten = vec![target_crate_ident.to_string()];
                rewritten.extend(segments[1..].iter().cloned());
                out.push(rewritten.join("::"));
            }
            _ => out.push(segments.join("::")),
        }
    }
    out
}

/// The generated check for a `state:`'s `holds:` clauses -- §5.4c's "type
/// invariants are assumed, never asserted", stopped being free.
///
/// Ply builds a value the only honest way it knows: through the type's own
/// constructor, honouring that constructor's own precondition and rejecting
/// rather than unwrapping a fallible one. Then it calls the type's own
/// operations on it, in a generated sequence, exactly as it already does to
/// reach a method deep in a type's state. Every clause is asserted twice
/// over: once on the value the constructor returned, and again after every
/// single operation. A clause that holds when a value is made and breaks
/// three operations later is the whole reason this is a sequence rather
/// than one call.
///
/// The counterexample marker names the clause and the step it broke at, so
/// a reader is told *which* promise about the structure failed and how many
/// operations in -- "the invariant broke" alone would send them to read the
/// whole type.
pub fn generate_invariant_test(
    ident: &str,
    type_path: &str,
    plan: &harness::ReceiverPlan,
    clauses: &[(syn::ExprClosure, String)],
    cases: u32,
    seed: &[u8; 32],
) -> Result<String> {
    if clauses.is_empty() {
        bail!("no `holds:` clause to check for `{type_path}`");
    }
    let label = type_path;
    let ctor_call = harness::last_two_segments(&plan.constructor);
    let ctor_args = call_args_for(&plan.ctor_params).join(", ");
    let ctor_pattern = value_pattern_for(&plan.ctor_params);
    let ctor_strategy = combined_strategy_expr_for(&plan.ctor_params)?;

    // One assertion block, emitted after construction and after every
    // operation. `step` is a literal the generated code prints, so a
    // failing case says how far into the sequence the structure went wrong.
    let assert_block = |step: &str, indent: &str| -> String {
        let mut out = String::new();
        for (i, (closure, text)) in clauses.iter().enumerate() {
            let call = harness::holds_clause_over(closure, "__ply_receiver")
                .to_token_stream()
                .to_string();
            let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!(
                "{indent}if !({call}) {{ \
                 eprintln!(\"PLY_HOLDS_CEX|{label}|clause={i}|after={{}}|{escaped}\", {step}); \
                 return Err(proptest::test_runner::TestCaseError::fail(\"a `holds:` clause is \
                 false for this value\")); }}\n"
            ));
        }
        out
    };

    let needs_mut = plan.operations.iter().any(|op| op.takes_mut_self);
    let mut_kw = if needs_mut { "mut " } else { "" };
    let ctor_expr = match plan.ctor_return {
        harness::CtorReturn::Bare => format!("{ctor_call}({ctor_args})"),
        harness::CtorReturn::ResultSelf => format!(
            "match {ctor_call}({ctor_args}) {{ Ok(__ply_ctor_ok) => __ply_ctor_ok, Err(_) => {{ \
             __ply_rejected.set(__ply_rejected.get() + 1); return \
             Err(proptest::test_runner::TestCaseError::reject(\"constructor returned Err\")); }} }}"
        ),
    };

    let mut body = String::new();
    body.push_str(&params_preamble(&plan.ctor_params)?);
    if let Some(ctor_requires) = &plan.ctor_requires {
        let cond = ctor_requires.to_token_stream().to_string();
        body.push_str(&format!(
            "if !({cond}) {{ __ply_rejected.set(__ply_rejected.get() + 1); return \
             Err(proptest::test_runner::TestCaseError::reject(\"constructor requires filter\")); \
             }}\n            "
        ));
    }
    body.push_str(&format!("let {mut_kw}__ply_receiver = {ctor_expr};\n"));
    // Reaching this line means a value really was built: every gate above
    // -- the constructor's own precondition, and its `Result` if it is
    // fallible -- let this case through. Counted separately from
    // `__ply_total` because the two answer different questions, and the
    // difference is the whole of what a run can honestly claim. A
    // constructor that rejects every draw leaves this at zero while
    // `__ply_total` reads 256, and a verdict read off the wrong one of
    // those reports 256 cases of evidence for a value that was never made.
    body.push_str("            __ply_checked.set(__ply_checked.get() + 1);\n");
    body.push_str(&assert_block("0usize", "            "));

    // The sequence. Every operation is a pooled one here -- there is no
    // checked method playing the part of operation zero, because there is
    // no checked method at all.
    let (seq_pattern, seq_strategy) = if plan.operations.is_empty() {
        (String::new(), String::new())
    } else {
        let mut patterns = Vec::new();
        let mut strategies = Vec::new();
        for (i, op) in plan.operations.iter().enumerate() {
            let (pattern, strategy, _) = op_pattern_strategy_preamble(&op.params, i)?;
            patterns.push(pattern);
            strategies.push(strategy);
        }
        (patterns.join(", "), strategies.join(", "))
    };
    if !plan.operations.is_empty() {
        body.push_str(&format!(
            "            let mut __ply_step = 0usize;\n            for (__ply_op_choice, \
             {seq_pattern}) in __ply_seq {{\n                __ply_step += 1;\n                \
             match __ply_op_choice {{\n"
        ));
        for (i, op) in plan.operations.iter().enumerate() {
            let call = harness::last_two_segments(&op.call_path);
            let op_args = call_args_for_prefixed(&op.params, i).join(", ");
            let recv_ref = if op.takes_mut_self {
                "&mut __ply_receiver"
            } else {
                "&__ply_receiver"
            };
            let full_args = if op_args.is_empty() {
                recv_ref.to_string()
            } else {
                format!("{recv_ref}, {op_args}")
            };
            let bind = op_pattern_strategy_preamble(&op.params, i)?.2;
            body.push_str(&format!(
                "                    {i} => {{ {bind}let _ = {call}({full_args}); }}\n"
            ));
        }
        body.push_str(
            "                    _ => unreachable!(\"__ply_op_choice is generated in \
             0..num_ops\"),\n                }\n",
        );
        body.push_str(&assert_block("__ply_step", "                "));
        body.push_str("            }\n");
    }

    let num_ops = plan.operations.len();
    let (outer_pattern, outer_strategy) = if plan.operations.is_empty() {
        (ctor_pattern, ctor_strategy)
    } else {
        (
            format!("({ctor_pattern}, __ply_seq)"),
            format!(
                "({ctor_strategy}, proptest::collection::vec((0u8..{num_ops}u8, {seq_strategy}), \
                 0..={max}usize))",
                max = plan.max_sequence_len
            ),
        )
    };
    let seed_literal = format!(
        "[{}]",
        seed.iter()
            .map(|b| format!("{b}u8"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let seed_hex = seed_hex(seed);

    Ok(format!(
        "    #[test]\n\
         \x20\x20\x20\x20fn ply_holds_{ident}() {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"PLY_FUZZ_SEED|{label}|{seed_hex}\");\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let mut __ply_runner = proptest::test_runner::TestRunner::new_with_rng(\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20proptest::test_runner::Config {{ cases: {cases}, failure_persistence: None, ..proptest::test_runner::Config::default() }},\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20proptest::test_runner::TestRng::from_seed(\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20proptest::test_runner::RngAlgorithm::ChaCha, &{seed_literal}),\n\
         \x20\x20\x20\x20\x20\x20\x20\x20);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_rejected = std::cell::Cell::new(0u32);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_total = std::cell::Cell::new(0u32);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_checked = std::cell::Cell::new(0u32);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_strategy = {outer_strategy};\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_outcome = __ply_runner.run(&__ply_strategy, |{outer_pattern}| {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_total.set(__ply_total.get() + 1);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20{body}\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(())\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}});\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_rej = __ply_rejected.get();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_tot = __ply_total.get();\n\
         \x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"PLY_HOLDS_STATS|{label}|checked={{}}|rejected={{}}|total={{}}\", __ply_checked.get(), __ply_rej, __ply_tot);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20match __ply_outcome {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(()) => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if __ply_tot > 0 && (__ply_rej as f64) / (__ply_tot as f64) > 0.5 {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"PLY_FUZZ_HIGH_REJECT|{label}|{{}}/{{}}\", __ply_rej, __ply_tot);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(proptest::test_runner::TestError::Abort(reason)) => {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"PLY_FUZZ_ABORT|{label}|{{}}|accepted={{}}|rejected={{}}\", reason, __ply_tot - __ply_rej, __ply_rej);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(e) => panic!(\"a `holds:` clause of `{label}` is false: {{}}\", e),\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20}}\n",
    ))
}

/// The module wrapper for [`generate_invariant_test`] -- the same job
/// [`wrap_fn_harness_module`] does for a function, minus everything only a
/// checked function needs. What it must still bring into scope is every
/// type the construction touches: the state type itself, its constructor's
/// own arguments, and the arguments of every operation the sequence calls.
/// `ident` is the caller's own name for this check, used for both the
/// module and the test inside it. Passed in rather than derived here: the
/// runner filters `cargo test` by module name, and a name derived twice
/// from different inputs drifted apart the moment two components could
/// promise things about the same type -- the filter then matched nothing,
/// and a check that never ran is not a check that passed.
pub fn wrap_invariant_harness_module(
    ident: &str,
    type_path: &str,
    plan: &harness::ReceiverPlan,
    target_crate_ident: &str,
    body: &str,
) -> String {
    let module_ident = ident;
    let mut out = format!(
        "#[cfg(test)]\nmod {module_ident}_holds_harness {{\n    #[allow(unused_imports)]\n    use {target_crate_ident}::{type_path};\n\
         \x20\x20\x20\x20#[allow(unused_imports)]\n    use {target_crate_ident}::*;\n"
    );
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    emitted.insert(format!("{target_crate_ident}::{type_path}"));
    let mut seen = std::collections::HashSet::new();
    let mut types = Vec::new();
    for p in &plan.ctor_params {
        collect_type_imports(&p.ty, target_crate_ident, &mut seen, &mut types);
    }
    for op in &plan.operations {
        for p in &op.params {
            collect_type_imports(&p.ty, target_crate_ident, &mut seen, &mut types);
        }
    }
    for full in types {
        if emitted.insert(full.clone()) {
            out.push_str(&format!("    #[allow(unused_imports)]\n    use {full};\n"));
        }
    }
    out.push('\n');
    out.push_str(body);
    out.push_str("}\n");
    out
}

pub fn wrap_fn_harness_module(
    cf: &ContractFn,
    target_crate_ident: &str,
    bodies: &[String],
) -> String {
    let module_ident = cf.ident();
    // Import exactly what `call_expr()` needs in scope: the whole path for
    // a free function (its bare final segment is then callable directly),
    // or the path *minus* its final segment for a method -- the type,
    // never the method itself, which is not an importable item at all
    // (`use crate::Bucket::new;` does not compile; `use crate::Bucket;`
    // plus calling `Bucket::new(..)` does). Added 2026-08-27: before this,
    // every method's harness crate failed to compile with an unresolved
    // import naming the method.
    let import_path = cf.import_path();
    let mut out = format!(
        "#[cfg(test)]\nmod {module_ident}_harness {{\n    #[allow(unused_imports)]\n    use {target_crate_ident}::{import_path};\n\
         \x20\x20\x20\x20#[allow(unused_imports)]\n    use {target_crate_ident}::*;\n"
    );
    // The glob import above (2026-09-01, plain-parameter seeding widening,
    // TODO.md): an `examples:` entry may reference a type -- `Opaque` in
    // `always_true(Opaque::default())`, say -- that never appears anywhere
    // in `cf`'s own resolved parameter types (`extra_type_imports` below
    // only ever sees a `RustType::UserTypeCtor`/`UserTypeFields`, and an
    // opaque type this scan could not build a value of, per `docs/review-
    // self-construction.md`'s rule 3, stays `Unsupported` forever). Before
    // this, only `test`/`fuzz` checks on a fn whose *own params* were all
    // recognised user types ever needed such a name in scope, so the
    // specific `use` list below was enough; opening `test` for an opaque
    // parameter with an `examples:`-only seed (`run_fn_checks`'s own
    // `test_unlocked_by_examples`) is the first shape where an example's
    // own literal source can name a type nothing else here ever resolves --
    // an explicit `use` several lines below always wins a name clash with
    // this glob (Rust's own shadowing rule for a glob vs. a named import),
    // so this never changes what any existing generated harness resolves a
    // name to.
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    emitted.insert(format!("{target_crate_ident}::{import_path}"));
    // Struct/enum parameters (2026-08-27): one `use` per type Ply itself
    // constructs -- assumed to sit at the target crate's own root, same as
    // every other bare struct/enum name this scan resolves
    // (`scan_crate_type_locations` indexes by bare name, not module path) --
    // except a cross-crate route (§5.4b's extension, defect 2, 2026-09-02),
    // whose own import is already a full, absolute path (`extra_type_
    // imports` resolves the difference; see `resolved_import`'s own doc).
    for full in extra_type_imports(cf, target_crate_ident) {
        if emitted.insert(full.clone()) {
            out.push_str(&format!("    #[allow(unused_imports)]\n    use {full};\n"));
        }
    }
    // A type the *contract text* names directly, rather than one reached
    // by walking a parameter or the receiver -- see
    // `contract_referenced_use_imports`'s own doc (docs/reach-measurement-2.md).
    // These are already full paths (`std::cmp::Ordering`,
    // `{target_crate_ident}::foo::Bar`), unlike `extra_type_imports`'s bare
    // names, so no additional prefix is added here.
    for full in contract_referenced_use_imports(cf, target_crate_ident) {
        if emitted.insert(full.clone()) {
            out.push_str(&format!("    #[allow(unused_imports)]\n    use {full};\n"));
        }
    }
    out.push('\n');
    for b in bodies {
        out.push_str(b);
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::discover_fn;

    fn discover(src: &str, name: &str) -> ContractFn {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, src).unwrap();
        discover_fn(&path, name).unwrap()
    }

    /// The composition task's own headline case (TODO.md, compprobe's
    /// `many`): a list of a user struct with an infallible constructor.
    /// Generates real, compiling proptest code -- never a `prop_map` that
    /// tries to build `Doc` directly (which `Doc` deriving no `Debug` would
    /// break, see `raw_user_type_strategy_expr`'s own doc for the measured
    /// compile error that shape produced before this fix): the strategy
    /// draws only a raw `u32` leaf per element, and the real `Doc::new`
    /// call happens in the preamble, via ordinary (non-proptest) code.
    #[test]
    fn a_list_of_a_user_struct_generates_a_real_construction_preamble_not_a_prop_map() {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let path = src_dir.join("lib.rs");
        std::fs::write(
            &path,
            r#"
pub struct Doc { n: u32 }
impl Doc { pub fn new(n: u32) -> Self { Doc { n } } }
#[ply::ensures(|result| *result >= 0)]
pub fn many(a: Vec<Doc>) -> i64 { a.len() as i64 }
"#,
        )
        .unwrap();
        let mut cf = discover_fn(&path, "many").unwrap();
        harness::enrich_contract_fn_user_types(&mut cf, dir.path(), &Default::default());
        assert!(cf.is_fuzz_supported(), "{:?}", cf.params[0].ty);
        let body =
            generate_fuzz_test(&cf, 8, &derive_seed("many", "")).unwrap_or_else(|e| panic!("{e}"));
        let strategy_line = body
            .lines()
            .find(|l| l.contains("__ply_strategy ="))
            .unwrap_or_else(|| panic!("no strategy line in generated body:\n{body}"));
        assert!(
            !strategy_line.contains("Doc::new"),
            "the constructor must never be called *inside* the proptest strategy line (that is \
             exactly the shape that fails to compile with `Doc doesn't implement Debug`) -- only \
             the raw u32 leaf may be drawn there:\n{strategy_line}"
        );
        assert!(
            body.contains("let a = ") && body.contains("Doc::new"),
            "the real Vec<Doc> must be built from the raw draw via an ordinary preamble \
             statement that calls the real constructor on each drawn leaf:\n{body}"
        );
    }

    /// Regression pin (task, 2026-08-27): the marker-precompute fix for the
    /// `String`/`Vec` moved-parameter defect must not break the far more
    /// common scalar case. `marker_display_expr`'s default arm returns the
    /// *bare variable reference* (`"x"`), relying on the surrounding
    /// `format!("{}", x)` call to `Display` it -- assigning that bare
    /// expression straight to a `String`-typed binding (`let _: String =
    /// x;`) is `error[E0308]: mismatched types` for every scalar-parameter
    /// fn, every one of them, the moment a precompute binding is
    /// `String`-typed but the expression is not. This is the actual defect
    /// this task's own `movedparam` e2e fixture caught (`scalar`, a plain
    /// `x: u32` fn wholly unrelated to `String`/`Vec`, failed to build).
    #[test]
    fn the_precomputed_marker_binding_for_a_plain_scalar_is_well_typed() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 { x.min(100) }
"#,
            "clamp",
        );
        let body = generate_fuzz_test(&cf, 256, &derive_seed("clamp", "")).unwrap();
        assert!(
            !body.contains(": String = x;"),
            "a bare scalar reference assigned directly to a String-typed binding does not \
             compile (expected String, found u32):\n{body}"
        );
    }

    // -- 2026-09-02: the branch-decided measurement (CLAUDE.md, "record
    // which branch of the promise actually decided each case"). The real
    // proof that evaluation order is preserved lives in the `orskewed`
    // e2e fixture (a running harness whose right-hand arm panics if forced
    // eagerly); these are the cheap, no-subprocess pins on the codegen
    // shape that produces it.

    /// A top-level `||` postcondition must generate the per-arm counters,
    /// the `if`/`else if` chain that decides which one, and the marker that
    /// reports the split back to `verify` -- never silently falling back to
    /// the plain boolean expression a promise with no `||` still uses.
    #[test]
    fn an_or_postcondition_generates_the_split_machinery() {
        let cf = discover(
            r#"
#[ply::ensures(|result| x < 100 || result.unwrap() == x)]
pub fn maybe_pass_through(x: u32) -> Option<u32> {
    if x < 100 { None } else { Some(x) }
}
"#,
            "maybe_pass_through",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("maybe_pass_through", "")).unwrap();
        assert!(
            body.contains("__ply_or_hit_0") && body.contains("__ply_or_hit_1"),
            "each arm needs its own counter:\n{body}"
        );
        assert!(
            body.contains("} else if"),
            "the split must be an `if`/`else if` chain -- the same left-to-right, \
             short-circuiting shape `||` itself has, never all arms evaluated up front:\n{body}"
        );
        assert!(
            body.contains("PLY_FUZZ_OR_SPLIT|maybe_pass_through|"),
            "the split must be reported back through its own marker:\n{body}"
        );
    }

    /// A promise with no top-level `||` at all must generate none of this --
    /// `flatten_top_level_or` refuses quietly (CLAUDE.md: "refusing
    /// quietly rather than guessing"), and the generated harness must stay
    /// byte-for-byte what it was before this feature existed.
    #[test]
    fn a_plain_postcondition_generates_none_of_the_split_machinery() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 { x.min(100) }
"#,
            "clamp",
        );
        let body = generate_fuzz_test(&cf, 256, &derive_seed("clamp", "")).unwrap();
        assert!(
            !body.contains("PLY_FUZZ_OR_SPLIT") && !body.contains("__ply_or_hit"),
            "a promise with no top-level `||` must generate no split machinery at all:\n{body}"
        );
    }

    // -- 2026-09-02: the degenerate-route guard (TODO.md, "the guard this
    // cannot ship without"). A route-built parameter is still an ordinary
    // `RustType::UserTypeCtor` -- the only thing new is `ReceiverPlan::route`
    // -- so these pin the *extra* codegen the guard adds, never a second
    // construction mechanism.

    fn discover_with_route(
        src: &str,
        fn_name: &str,
        type_name: &str,
        route_fn: &str,
    ) -> ContractFn {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let path = src_dir.join("lib.rs");
        std::fs::write(&path, src).unwrap();
        let mut cf = discover_fn(&path, fn_name).unwrap();
        let mut routes = harness::RouteTable::new();
        routes.insert(type_name.to_string(), route_fn.to_string());
        let refused = harness::enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert!(refused.is_empty(), "{refused:?}");
        cf
    }

    /// The defect the false-green work ended on (TODO.md, 2026-09-02): a
    /// type whose only way in is a no-argument constructor was checked
    /// against one value, and the mutator that would have varied it was left
    /// out of the sequence because Ply "cannot build a value for" its
    /// argument -- a plain enum with no data in any variant.
    ///
    /// The inconsistency is what makes it a defect rather than a missing
    /// feature: the identical enum builds fine as an ordinary top-level
    /// parameter. What was missing is not enum support, it is that an
    /// operation's own parameters were never put through the same
    /// resolution the checked call's parameters get -- so they stayed
    /// `Unsupported` and the operation was filtered out of the pool.
    ///
    /// Asserts the generated harness both binds a real value for the
    /// argument and calls the mutator with it, because the operation
    /// appearing in the pool while its argument is never bound would fail
    /// to compile -- a defect that trades a silent gap for a loud one.
    #[test]
    fn an_operation_taking_a_plain_enum_is_called_rather_than_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            r#"
#[derive(Clone, Copy)]
pub enum Flag { Ready, Busy, Failed }
#[derive(Clone, Copy, Default)]
pub struct FlagSet(u8);
impl FlagSet {
    pub const fn new() -> Self { FlagSet(0) }
    pub fn insert(&mut self, f: Flag) { let _ = f; }
    #[ply::ensures(|result| *result <= 3)]
    pub fn len(&self) -> usize { self.0.count_ones() as usize }
}
"#,
        )
        .unwrap();
        let mut cf = harness::discover_method_with_receiver(
            dir.path(),
            "FlagSet::len",
            &harness::RouteTable::new(),
        )
        .expect("`FlagSet::len` is a method on a type with a public constructor");
        let refused = harness::enrich_contract_fn_user_types(
            &mut cf,
            dir.path(),
            &harness::RouteTable::new(),
        );
        assert!(refused.is_empty(), "{refused:?}");

        let plan = cf
            .receiver
            .as_ref()
            .expect("`FlagSet::len` is a method, so it has a receiver plan");
        assert!(
            plan.operations
                .iter()
                .any(|op| op.call_path.ends_with("::insert")),
            "`insert` takes a plain enum, which Ply builds fine as an ordinary parameter, so \
             it belongs in the sequence pool rather than the excluded list. Excluded: {:?}",
            plan.excluded_operations
        );

        let body = generate_fuzz_test(&cf, 64, &derive_seed("FlagSet::len", "")).unwrap();
        assert!(
            body.contains("FlagSet::insert("),
            "the generated sequence must actually call the mutator:\n{body}"
        );
        assert!(
            body.contains("Flag::Busy"),
            "the mutator's own argument must be built, not left unbound -- an operation in \
             the pool whose argument is never bound does not compile:\n{body}"
        );
    }

    /// The other half of the same decision, and the reason moving it needed
    /// its own pin: an operation whose argument is *genuinely* unbuildable
    /// must still be left out of the pool and still be named, with its
    /// reason. Admitting everything would trade the old silent gap for a
    /// harness that does not compile, which is a worse failure than the one
    /// being fixed.
    ///
    /// A filesystem path is the case to hold this against -- Ply's own reach
    /// measurement records paths as deliberately unbuilt, so this stays true
    /// as the supported-type list grows.
    #[test]
    fn an_operation_whose_argument_really_cannot_be_built_is_still_named() {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            r#"
#[derive(Clone, Copy, Default)]
pub struct Log(u8);
impl Log {
    pub const fn new() -> Self { Log(0) }
    pub fn write_to(&mut self, at: std::path::PathBuf) { let _ = at; }
    #[ply::ensures(|result| *result <= 3)]
    pub fn count(&self) -> usize { self.0 as usize }
}
"#,
        )
        .unwrap();
        let mut cf = harness::discover_method_with_receiver(
            dir.path(),
            "Log::count",
            &harness::RouteTable::new(),
        )
        .expect("`Log::count` is a method on a type with a public constructor");
        let _ = harness::enrich_contract_fn_user_types(
            &mut cf,
            dir.path(),
            &harness::RouteTable::new(),
        );

        let plan = cf.receiver.as_ref().expect("a method has a receiver plan");
        assert!(
            !plan
                .operations
                .iter()
                .any(|op| op.call_path.ends_with("::write_to")),
            "a path is not a type Ply builds values of, so `write_to` must not be pooled"
        );
        let named = plan
            .excluded_operations
            .iter()
            .find(|op| op.call_path.ends_with("::write_to"))
            .expect("an operation left out must be named, never merely absent");
        assert!(
            named.reason.contains("at"),
            "the reason must name the argument that could not be built: {}",
            named.reason
        );
    }

    /// A route-built top-level parameter whose type derives `Debug` gets a
    /// running set that records every distinct value actually built, and
    /// the harness reports the split unconditionally once the run ends --
    /// never gated on the count turning out degenerate, the same
    /// "print always, mark only when it collapses" shape `PLY_FUZZ_OR_SPLIT`
    /// already follows.
    #[test]
    fn a_debug_deriving_route_built_parameter_counts_distinct_values() {
        let cf = discover_with_route(
            r#"
#[derive(Debug)]
pub struct Handle { id: u32 }
pub fn open_handle(id: u32) -> Handle { Handle { id } }
#[ply::ensures(|result| *result >= 0)]
pub fn use_handle(h: &Handle) -> i64 { h.id as i64 }
"#,
            "use_handle",
            "Handle",
            "open_handle",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("use_handle", "")).unwrap();
        assert!(
            body.contains("BTreeSet::<String>::new()"),
            "a debug-derivable route-built parameter needs a set to count distinct values \
             into:\n{body}"
        );
        assert!(
            body.contains("format!(\"{:?}\", h)"),
            "each case's built value must be recorded by its Debug text:\n{body}"
        );
        assert!(
            body.contains("PLY_ROUTE_DISTINCT|use_handle|h|open_handle|"),
            "the split must be reported back through its own marker, naming the parameter and \
             the declared route:\n{body}"
        );
    }

    /// A route-built parameter whose type does **not** derive `Debug` cannot
    /// be printed or compared by code Ply generates from outside the crate
    /// -- the guard says so plainly instead of guessing a count from
    /// nothing (module doc: "where the type cannot be compared or printed,
    /// say so rather than guessing a number").
    #[test]
    fn a_non_debug_route_built_parameter_discloses_it_cannot_count_distinct_values() {
        let cf = discover_with_route(
            r#"
pub struct Handle { id: u32 }
pub fn open_handle(id: u32) -> Handle { Handle { id } }
#[ply::ensures(|result| *result >= 0)]
pub fn use_handle(h: &Handle) -> i64 { h.id as i64 }
"#,
            "use_handle",
            "Handle",
            "open_handle",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("use_handle", "")).unwrap();
        assert!(
            !body.contains("BTreeSet::<String>::new()"),
            "there is nothing to count into when the type cannot be printed:\n{body}"
        );
        assert!(
            body.contains("PLY_ROUTE_UNPRINTABLE|use_handle|h|open_handle|"),
            "the run must say plainly that it could not count distinct values, rather than \
             staying silent:\n{body}"
        );
    }

    /// An ordinary constructor-built parameter (rule 1, no route declared at
    /// all) must generate none of this -- the guard exists for the one
    /// failure the compiler cannot catch on its own (an author's route
    /// ignoring its inputs), and a found constructor is not that.
    #[test]
    fn a_constructor_built_parameter_with_no_route_generates_no_distinct_tracking() {
        let cf = discover_with_route(
            r#"
#[derive(Debug)]
pub struct TicketPool { capacity: u32 }
impl TicketPool { pub fn new(capacity: u32) -> Self { TicketPool { capacity } } }
#[ply::ensures(|result| *result >= 0)]
pub fn doubled(p: TicketPool) -> i64 { p.capacity as i64 * 2 }
"#,
            "doubled",
            // No route declared for `TicketPool` at all -- routes is keyed
            // by a type nothing here names, so `enrich` resolves `p` via
            // rule 1's own constructor scan, exactly as if `discover_with_route`
            // were never called with a route in the first place.
            "Unrelated",
            "unrelated_fn",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("doubled", "")).unwrap();
        assert!(
            !body.contains("PLY_ROUTE_DISTINCT") && !body.contains("PLY_ROUTE_UNPRINTABLE"),
            "a constructor Ply found on its own is not a declared route, and must not be \
             flagged as needing this guard:\n{body}"
        );
    }

    /// docs/reach-measurement-2.md: a contract that *names* a type -- not a
    /// parameter, not the return type, just a name written in the
    /// `#[ply::ensures]`/`#[ply::requires]` text itself -- failed to
    /// compile with `error[E0433]: cannot find type Ordering in this
    /// scope`, because `wrap_fn_harness_module` only ever imports the
    /// checked fn's own path and the types `extra_type_imports` finds by
    /// walking *parameters and the receiver* -- never anything the contract
    /// text alone refers to. This fixture reproduces the defect without
    /// touching the return-type gate at all (`Ordering` here is not the
    /// return type or a parameter -- just a name the postcondition reads --
    /// so it is real regardless of the separate gate question): the crate
    /// under test imports `std::cmp::Ordering` at the top of the file and
    /// the contract names it, and nothing about the checked fn's signature
    /// should stop the harness from seeing what the file itself can see.
    #[test]
    fn a_contract_naming_a_type_used_nowhere_in_the_signature_still_gets_its_own_import() {
        let cf = discover(
            r#"
use std::cmp::Ordering;
#[ply::ensures(|result| *result || Ordering::Equal == Ordering::Equal)]
pub fn f(x: u32) -> bool { x > 0 }
"#,
            "f",
        );
        let fuzz_body = generate_fuzz_test(&cf, 8, &derive_seed("f", "")).unwrap();
        let module = wrap_fn_harness_module(&cf, "target_crate", &[fuzz_body]);
        assert!(
            module.contains("use std::cmp::Ordering;"),
            "a contract naming a type the checked fn's own file already imports must bring \
             that same import into the generated harness module, or the harness fails to \
             compile with \"cannot find type `Ordering` in this scope\" even though the type \
             is right there in scope for the real function:\n{module}"
        );
    }

    /// Defect 2's codegen half (2026-09-02): a cross-crate route's own
    /// import is already a full, absolute path (`std::ffi::OsString`), not
    /// a bare name this crate declares -- `wrap_fn_harness_module` must
    /// bring it into scope verbatim, never prefixed with the target
    /// crate's own name the way every ordinary (in-crate) parameter type
    /// is. Getting this wrong generates `use target_crate::std::ffi::
    /// OsString;`, which does not compile at all.
    #[test]
    fn a_cross_crate_routes_import_is_never_prefixed_with_the_target_crate() {
        let mut cf = discover(
            r#"
#[ply::ensures(|result| *result >= 0)]
pub fn use_os_string(o: std::ffi::OsString) -> i64 { o.len() as i64 }
"#,
            "use_os_string",
        );
        let dir = tempfile::tempdir().unwrap();
        let mut routes = harness::RouteTable::new();
        routes.insert(
            "OsString".to_string(),
            "std::ffi::OsString::from(String)".to_string(),
        );
        let refused = harness::enrich_contract_fn_user_types(&mut cf, dir.path(), &routes);
        assert!(refused.is_empty(), "{refused:?}");
        assert!(cf.is_fuzz_supported(), "{:?}", cf.params[0].ty);
        let fuzz_body = generate_fuzz_test(&cf, 8, &derive_seed("use_os_string", "")).unwrap();
        let module = wrap_fn_harness_module(&cf, "target_crate", &[fuzz_body]);
        assert!(
            module.contains("use std::ffi::OsString;"),
            "a cross-crate route's own type must be imported by its real, absolute path:\n{module}"
        );
        assert!(
            !module.contains("target_crate::std"),
            "a cross-crate route's import must never be prefixed with the target crate's own \
             name -- that path does not exist and does not compile:\n{module}"
        );
        assert!(
            module.contains("OsString::from("),
            "the declared route's own function must still be called:\n{module}"
        );
    }

    #[test]
    fn generates_a_fuzz_test_for_a_scalar_fn() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 { x.min(100) }
"#,
            "clamp",
        );
        let body = generate_fuzz_test(&cf, 256, &derive_seed("clamp", "")).unwrap();
        assert!(body.contains("fn ply_fuzz_clamp()"));
        assert!(body.contains("cases: 256"));
        assert!(body.contains("PLY_FUZZED_CEX|clamp|"));
        assert!(body.contains("TestRunner::new_with_rng"));
    }

    /// vetting 004 finding 4: the same source gave a different verdict on
    /// six of six runs because the runner's RNG came from entropy. The
    /// generated harness must pin its seed, and the same input must always
    /// derive the same one.
    #[test]
    fn the_generated_harness_pins_a_seed_and_the_same_source_derives_the_same_one() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 { x.min(100) }
"#,
            "clamp",
        );
        let seed = derive_seed("clamp", "|result|*result == x");
        let body = generate_fuzz_test(&cf, 256, &seed).unwrap();
        assert!(
            body.contains("TestRng::from_seed"),
            "the runner must be built from a recorded seed, never from entropy:\n{body}"
        );
        assert!(
            body.contains("failure_persistence: None"),
            "proptest's own persisted-failure replay is a second source of run-to-run \
             difference, and must be off for the run to be reproducible from the seed alone:\n{body}"
        );
        assert_eq!(
            derive_seed("clamp", "|result|*result == x"),
            seed,
            "identical source must always derive an identical seed"
        );
        assert_ne!(
            derive_seed("clamp", "|result|*result >= x"),
            seed,
            "a changed contract gets its own draw sequence"
        );
        assert_ne!(
            derive_seed("other", "|result|*result == x"),
            seed,
            "two fns in one run must not sample the same sequence"
        );
    }

    #[test]
    fn a_seed_round_trips_through_its_hex_form_and_a_malformed_one_is_refused() {
        let seed = derive_seed("f", "c");
        assert_eq!(seed_from_hex(&seed_hex(&seed)), Some(seed));
        assert_eq!(seed_hex(&seed).len(), 64);
        assert_eq!(seed_from_hex("abc"), None);
        assert_eq!(seed_from_hex(&"z".repeat(64)), None);
    }
    #[test]
    fn btree_set_param_builds_a_strategy_without_panicking() {
        let cf = discover(
            r#"
use std::collections::BTreeSet;
#[ply::ensures(|result| *result == xs.len() as u32)]
pub fn count(xs: &BTreeSet<u8>) -> u32 { xs.len() as u32 }
"#,
            "count",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("count", "")).unwrap();
        assert!(body.contains("btree_set"));
        assert!(
            body.contains("&xs"),
            "by-ref param must be called by reference:\n{body}"
        );
    }

    #[test]
    fn requires_becomes_a_reject_not_an_assert() {
        let cf = discover(
            r#"
#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == x + 1)]
pub fn safe_increment(x: u32) -> u32 { x + 1 }
"#,
            "safe_increment",
        );
        let body = generate_fuzz_test(&cf, 256, &derive_seed("clamp", "")).unwrap();
        assert!(body.contains("TestCaseError::reject"));
        assert!(
            body.contains("x < u32 :: MAX")
                || body.contains("x<u32::MAX")
                || body.contains("x < u32::MAX")
        );
    }

    /// Defect B: `v: Vec<u8>` is passed by value, so it has been moved into
    /// the call by the time the postcondition reads `v.len()` -- the exact
    /// shape a compile failure the task's repro used. Before the refusal
    /// existed, this reached codegen and produced a harness with
    /// `error[E0382]: borrow of moved value: `v``.
    #[test]
    fn a_postcondition_reading_a_moved_by_value_vec_is_refused_by_name() {
        let cf = discover(
            r#"
#[ply::requires(v.len() <= 4)]
#[ply::ensures(|result| *result as usize >= v.len())]
pub fn vector(v: Vec<u8>) -> u32 { v.len() as u32 }
"#,
            "vector",
        );
        let refused = moved_param_read_in_ensures(&cf);
        assert_eq!(
            refused.map(|p| p.name.as_str()),
            Some("v"),
            "the moved parameter must be named, not merely detected"
        );
        let err = generate_fuzz_test(&cf, 32, &derive_seed("vector", "")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("V0506"), "{msg}");
        assert!(
            msg.contains("moved"),
            "the message must say what actually goes wrong: {msg}"
        );
        assert!(
            generate_direct_contract_cases(&cf).is_empty(),
            "the `test` tier's direct cases must refuse the same shape, not just `fuzz`"
        );
    }

    /// The construct `old()` exists for: reading a by-value parameter's
    /// *entry* value is fine, because `old(v)` is captured before the call,
    /// while `v` still exists.
    #[test]
    fn old_of_a_moved_by_value_param_is_not_refused() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result as usize == old(v).len())]
pub fn consume(v: Vec<u8>) -> u32 { v.len() as u32 }
"#,
            "consume",
        );
        assert!(
            moved_param_read_in_ensures(&cf).is_none(),
            "old(v) reads the entry value, before the move -- it must not be refused"
        );
        assert!(generate_fuzz_test(&cf, 32, &derive_seed("consume", "")).is_ok());
    }

    /// A by-*reference* parameter is only ever borrowed for the call, never
    /// moved -- reading it afterward in the postcondition is completely
    /// ordinary and must never be refused, even though its underlying type
    /// (`Vec<u8>`) would move if taken by value.
    #[test]
    fn a_by_reference_vec_param_is_never_refused() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result as usize >= v.len())]
pub fn vector_ref(v: &Vec<u8>) -> u32 { v.len() as u32 }
"#,
            "vector_ref",
        );
        assert!(moved_param_read_in_ensures(&cf).is_none());
    }

    /// A `Copy` scalar parameter read after the call is completely ordinary
    /// (this is `oldvalue`'s own `bump` shape without `old()`, and every
    /// M3/M4 fixture that reads a plain `x: u32` post-call): it was copied
    /// into the call, so the original is untouched.
    #[test]
    fn a_copy_scalar_param_read_after_the_call_is_never_refused() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result >= x)]
pub fn scalar(x: u32) -> u32 { x + 1 }
"#,
            "scalar",
        );
        assert!(moved_param_read_in_ensures(&cf).is_none());
    }

    #[test]
    fn renders_one_example_as_a_plain_assert_test() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result <= 100)]
pub fn clamp(x: u32) -> u32 { if x > 100 { 100 } else { x } }
"#,
            "clamp",
        );
        let body = generate_example_test(&cf, 1, "clamp(150) == 100").unwrap();
        assert!(body.contains("fn ply_example_clamp_01()"));
        assert!(body.contains("clamp (150) == 100") || body.contains("clamp(150) == 100"));
    }

    /// Ply's own refusal for a shape `fuzz`/`bounded` cannot build tells the
    /// user to "declare `test` instead, with an `examples:` entry, to run
    /// the concrete case directly". Doing exactly that on a *method* used to
    /// break: the checked function's own path (`Type::method`, exactly what
    /// every fixture exercising this codegen was missing -- every one of
    /// them used a free function) was spliced verbatim into the generated
    /// test's name, so `fn ply_example_Type::method_01()` is not a legal
    /// Rust identifier at all -- the harness crate fails with `error:
    /// invalid path separator in function definition`. Nearly everything in
    /// a real library is a method, so the escape hatch Ply itself recommends
    /// was broken for most of the cases it recommends it for.
    #[test]
    fn the_generated_example_test_name_is_never_a_qualified_path() {
        // A nested-module free function reproduces the same `::`-qualified
        // `path` a method has (`inner::helper`, exactly the shape
        // `Type::method` takes), without this low-level unit test also
        // having to satisfy the separate "can Ply build a receiver value"
        // gate `discover_fn` applies to an actual method -- the codegen bug
        // this pins is purely about splicing a `::`-qualified path into an
        // identifier, and is agnostic to *why* the path contains one. The
        // real motivating case (confirmed by hand against `semver`'s
        // `Version::cmp_precedence`, and pinned end-to-end by the
        // `methodexampletest` fixture) is a method.
        let cf = discover(
            r#"
pub mod inner {
    #[ply::ensures(|result| *result)]
    pub fn helper() -> bool { true }
}
"#,
            "inner::helper",
        );
        let body = generate_example_test(&cf, 1, "true").unwrap();
        let wrapped = format!("mod m {{\n{body}\n}}\n");
        assert!(
            syn::parse_str::<syn::File>(&wrapped).is_ok(),
            "a checked function's qualified path (`Type::method`) must never be spliced \
             verbatim into the generated test's name -- `::` is not legal inside a Rust \
             identifier, so the harness crate fails to build with `error: invalid path \
             separator in function definition`:\n{body}"
        );
        assert!(
            !body.contains("::"),
            "the generated test name must use a safe identifier (see `ContractFn::ident`), \
             never the raw `::`-qualified path: {body}"
        );
    }

    /// Every generated example test must be *valid Rust*, whatever the user
    /// wrote in `examples`. The entry is echoed into the assert's own
    /// failure message, and an entry containing a `"` (a perfectly ordinary
    /// one -- `f(0) == "zero"` was the 2026-08-24 M4 review's own D1 probe)
    /// used to close that message's string literal early, so the harness
    /// crate failed to build with a *syntax* error in Ply's own generated
    /// code -- burying the user's real mistake under a compiler error that
    /// points at a file they never wrote.
    #[test]
    fn an_example_containing_a_quote_is_escaped_in_the_assert_message() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == 0)]
pub fn greet(x: u32) -> u32 { x }
"#,
            "greet",
        );
        let body = generate_example_test(&cf, 1, r#"greet(0) == "zero""#).unwrap();
        assert!(
            body.contains(r#"`greet(0) == \"zero\"` does not hold"#),
            "the example text is echoed into a Rust string literal, so its quotes must be escaped -- \
             unescaped, the literal closes early and the harness fails to build with a syntax error \
             in Ply's own generated file:\n{body}"
        );
    }

    /// §5.4a: `old(expr)` is "the value `expr` had on entry", and the spec
    /// is explicit about how a generated test/fuzz harness must honour it --
    /// "evaluate `expr` before the call and substitute the snapshot". It did
    /// not: the clause went into the generated test exactly as written, so
    /// the harness called a function named `old` that exists nowhere, the
    /// harness crate failed to compile, and the whole check came back as an
    /// internal tool error naming a compiler message instead of the clause.
    #[test]
    fn a_before_value_in_an_ensures_is_read_before_the_call_not_left_as_a_call_to_old() {
        let cf = discover(
            r#"
#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == old(x) + 1)]
pub fn bump(x: u32) -> u32 { x + 1 }
"#,
            "bump",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("bump", "")).unwrap();
        assert!(
            !body.replace(' ', "").contains("old("),
            "the generated harness must not call a function named `old` -- there is no such \
             function, and the harness crate fails to compile with \"cannot find function `old` \
             in this scope\":\n{body}"
        );
        let snapshot = body
            .find("let __ply_old_0")
            .expect("the entry value must be read into a binding of its own:\n");
        let call = body
            .find("let __ply_call_result")
            .expect("generated harness always binds the call result");
        assert!(
            snapshot < call,
            "the entry value has to be read *before* the call, or it is not the entry value at \
             all:\n{body}"
        );
    }

    /// The same construct on the `test` tier's generated concrete cases,
    /// which are compiled into the same harness crate: one of them left as
    /// a call to `old` breaks the build for every check on the function.
    #[test]
    fn a_before_value_is_read_before_the_call_in_generated_concrete_cases_too() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result >= old(x))]
pub fn bump(x: u32) -> u32 { x.saturating_add(1) }
"#,
            "bump",
        );
        let cases = generate_direct_contract_cases(&cf);
        assert!(!cases.is_empty(), "expected generated concrete cases");
        assert!(
            !cases.replace(' ', "").contains("old("),
            "no generated case may call a function named `old` -- one that does breaks the build \
             for every check on the function:\n{cases}"
        );
        let snapshot = cases.find("let __ply_old_0").expect("entry value binding");
        let call = cases.find("let __ply_call_result").expect("call binding");
        assert!(snapshot < call, "read the entry value first:\n{cases}");
    }

    #[test]
    fn generates_direct_contract_cases_for_a_scalar_fn() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 { x.min(100) }
"#,
            "clamp",
        );
        let cases = generate_direct_contract_cases(&cf);
        assert!(cases.contains("fn ply_direct_clamp_00()"));
        assert!(cases.contains("0u32"));
        assert!(cases.contains("u32::MAX"));
    }

    // -- the NaN/infinity decision (task, 2026-08-27), pinned so reversing
    // it fails a test rather than silently reintroducing a false
    // counterexample. §5.4c's own words for why this matters: a generated
    // NaN makes almost any postcondition comparison false, which would
    // report a broken promise on an input the real program may never
    // produce.

    /// This is the pin: reverting `strategy_expr`'s float arm to a bare
    /// `proptest::prelude::any::<f64>()` (which samples NaN and infinity
    /// along with everything else) makes this test fail, because the
    /// finite-only class flags this asserts on would no longer appear in
    /// the generated source.
    #[test]
    fn float_strategy_excludes_nan_and_infinity_by_default() {
        for ty in [RustType::F32, RustType::F64] {
            let expr = strategy_expr(&ty).unwrap();
            for finite_class in ["POSITIVE", "NEGATIVE", "NORMAL", "SUBNORMAL", "ZERO"] {
                assert!(
                    expr.contains(finite_class),
                    "{:?}'s strategy must sample the `{finite_class}` class -- every ordinary \
                     finite value, one sign or the other -- got: {expr}",
                    ty
                );
            }
            for excluded_class in ["INFINITE", "QUIET_NAN", "SIGNALING_NAN"] {
                assert!(
                    !expr.contains(excluded_class),
                    "{:?}'s strategy must NOT sample `{excluded_class}` by default -- a generated \
                     NaN or infinity would report a false counterexample on an input the real \
                     program may never produce: {expr}",
                    ty
                );
            }
        }
    }

    /// The `test` tier's own boundary literals must respect the same
    /// finite-only decision: `NAN`/`INFINITY`/`NEG_INFINITY` are not
    /// boundary values Ply generates for a direct contract case either.
    #[test]
    fn float_boundary_literals_are_finite_only() {
        for ty in [RustType::F32, RustType::F64] {
            let lits = boundary_literals(&ty);
            assert!(!lits.is_empty(), "{:?}", ty);
            for lit in &lits {
                assert!(
                    !lit.contains("NAN") && !lit.contains("INFINITY"),
                    "a boundary literal for {:?} must be finite: {lits:?}",
                    ty
                );
            }
        }
    }

    #[test]
    fn generates_a_fuzz_test_for_a_float_param_and_names_the_finite_only_strategy() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result >= x)]
pub fn increment(x: f64) -> f64 { x + 1.0 }
"#,
            "increment",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("increment", "")).unwrap();
        assert!(body.contains("fn ply_fuzz_increment()"));
        assert!(body.contains("proptest::num::f64::NORMAL"));
        assert!(!body.contains("QUIET_NAN"));
    }

    // -- receiver construction (docs/review-self-construction.md's "fourth
    // option", 2026-08-27): a method whose `ContractFn::receiver` is `Some`
    // gets a constructor call plus a bounded operation sequence spliced into
    // its generated fuzz test.

    /// `fn_path` must be spelled `m::Type::method` -- this helper always
    /// writes the fixture source to `src/m.rs`, matching that one module
    /// segment.
    fn discover_receiver(src: &str, fn_path: &str) -> ContractFn {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("m.rs"), src).unwrap();
        harness::discover_method_with_receiver(dir.path(), fn_path, &Default::default()).unwrap()
    }

    #[test]
    fn generates_a_constructor_call_and_sequence_loop_for_a_receiver_method() {
        let cf = discover_receiver(
            r#"
pub struct Bucket { cap: u32 }
impl Bucket {
    pub fn new(cap: u32) -> Self { Bucket { cap } }
    #[ply::ensures(|result| *result <= 1_000_000)]
    pub fn capacity(&self) -> u32 { self.cap }
}
"#,
            "m::Bucket::capacity",
        );
        assert!(cf.receiver.is_some());
        let body = generate_fuzz_test(&cf, 32, &derive_seed("capacity", "")).unwrap();
        assert!(
            body.contains("let __ply_receiver = Bucket::new"),
            "the receiver must be built by calling the type's own constructor:\n{body}"
        );
        assert!(
            body.contains("for (__ply_op_choice, "),
            "a bounded sequence of the type's own operations must be spliced in before the \
             checked call:\n{body}"
        );
        assert!(
            body.contains("Bucket::capacity(&__ply_receiver)"),
            "the checked call itself must pass the built receiver:\n{body}"
        );
    }

    /// The pin the task asked for: a generated receiver is only ever built
    /// by calling the type's own code, never a struct literal -- so nobody
    /// later "improves" this into field-filling. Checked on generated
    /// source text directly, not merely on which function was called, so a
    /// future change that reintroduces a literal fails this test rather
    /// than passing quietly.
    #[test]
    fn a_generated_receiver_never_contains_a_struct_literal() {
        let cf = discover_receiver(
            r#"
pub struct Meter { n: std::cell::Cell<u32> }
impl Meter {
    pub fn new() -> Self { Meter { n: std::cell::Cell::new(0) } }
    pub fn bump(&self, amount: u32) -> u32 { self.n.set(self.n.get() + amount); self.n.get() }
    #[ply::ensures(|result| *result < 1_000_000)]
    pub fn spend(&self, amount: u32) -> u32 { self.n.set(self.n.get() - amount); self.n.get() }
}
"#,
            "m::Meter::spend",
        );
        let body = generate_fuzz_test(&cf, 32, &derive_seed("spend", "")).unwrap();
        assert!(
            !body.contains("Meter {") && !body.contains("Meter{"),
            "the generated receiver must be built by calling `Meter`'s own code (`Meter::new`), \
             never by writing a `Meter {{ .. }}` struct literal:\n{body}"
        );
        // The sibling operation `bump` shares `spend`'s own shape and must
        // be pooled alongside it (harness::tests already pins the plan
        // itself; this pins that codegen actually calls it).
        assert!(
            body.contains("Meter::bump(&__ply_receiver"),
            "a same-shape sibling operation must be callable from the sequence:\n{body}"
        );
    }

    /// docs/review-caveats.md N3, codegen half: a `&mut self` sibling
    /// operation whose own shape differs from the checked method's must
    /// still be spliced into the sequence, borrowed `&mut`, with its own
    /// (prefixed) argument names -- never left out for either reason.
    #[test]
    fn a_mut_self_different_shape_sibling_is_pooled_and_borrowed_mutably() {
        let cf = discover_receiver(
            r#"
pub struct Acc { n: u32 }
impl Acc {
    pub fn new() -> Self { Acc { n: 0 } }
    pub fn add(&mut self, k: u32) -> u32 { self.n += k; self.n }
    #[ply::ensures(|result| *result < 5)]
    pub fn get(&self) -> u32 { self.n }
}
"#,
            "m::Acc::get",
        );
        let body = generate_fuzz_test(&cf, 32, &derive_seed("get", "")).unwrap();
        assert!(
            body.contains("let mut __ply_receiver = Acc::new"),
            "any `&mut self` operation in the pool means the receiver binding itself must be \
             `mut`:\n{body}"
        );
        assert!(
            body.contains("Acc::add(&mut __ply_receiver"),
            "a `&mut self` sibling of a different shape must still be called, borrowed \
             mutably:\n{body}"
        );
    }

    /// docs/review-caveats.md N2: the constructor's own `#[ply::requires]`
    /// must gate the arguments Ply generates for it, exactly like the
    /// checked call's own `requires` already gates its own arguments.
    #[test]
    fn a_constructors_own_requires_gates_its_generated_arguments() {
        let cf = discover_receiver(
            r#"
pub struct Gauge { n: u32 }
impl Gauge {
    #[ply::requires(n > 0)]
    pub fn new(n: u32) -> Self { Gauge { n } }
    #[ply::ensures(|result| *result >= 0)]
    pub fn value(&self) -> u32 { self.n }
}
"#,
            "m::Gauge::value",
        );
        let body = generate_fuzz_test(&cf, 32, &derive_seed("value", "")).unwrap();
        assert!(
            body.contains("if !(n > 0 as u32)")
                || body.contains("if !(n > 0u32)")
                || body.contains("if !(n > 0)"),
            "the constructor's own precondition must be rendered as a rejection filter before \
             the receiver is built:\n{body}"
        );
        let ctor_pos = body.find("let __ply_receiver = Gauge::new").unwrap();
        let requires_pos = body.find("constructor requires filter").unwrap();
        assert!(
            requires_pos < ctor_pos,
            "the constructor's precondition must be checked *before* the constructor is called, \
             not after:\n{body}"
        );
    }

    /// docs/review-caveats.md N2, second half: the checked method's own
    /// `#[ply::requires]` must gate *every* call the sequence makes to it,
    /// not only the final one -- operation zero (a repeat of the checked
    /// method itself) must be wrapped in the same precondition check.
    #[test]
    fn the_checked_methods_own_requires_gates_its_repeats_inside_the_sequence() {
        let cf = discover_receiver(
            r#"
pub struct Thing { n: u32 }
impl Thing {
    pub fn new() -> Self { Thing { n: 0 } }
    #[ply::requires(k <= 10)]
    #[ply::ensures(|result| *result <= 10)]
    pub fn set(&self, k: u32) -> u32 { k }
}
"#,
            "m::Thing::set",
        );
        let body = generate_fuzz_test(&cf, 32, &derive_seed("set", "")).unwrap();
        // Arm zero (the checked method's own repeat) must itself be guarded
        // by the same precondition text used for the final call -- looking
        // for the arm's own guarded-call shape rather than a raw substring
        // match on the whole body, so this fails if the guard is missing
        // even though the *final* call's own filter (elsewhere in the body)
        // still contains the same text.
        assert!(
            body.contains("0 => { if k <= 10u32 { let _ = Thing::set(&__ply_receiver, k); } }")
                || body.contains("0 => { if k <= 10 { let _ = Thing::set(&__ply_receiver, k); } }"),
            "the sequence's own repeat of the checked method must be gated by its own \
             precondition, never called out of contract:\n{body}"
        );
    }

    /// A zero-parameter constructor and a zero-parameter checked method
    /// (the shape `Meter::new()`/a no-arg operation would take) must not
    /// regress the fix already pinned for a free zero-parameter fn --
    /// `Just(())` for the constructor slot, `_` for its pattern, and no
    /// stray trailing comma in the checked call's own argument list.
    #[test]
    fn a_zero_arg_constructor_and_zero_arg_checked_method_both_compile_cleanly() {
        let cf = discover_receiver(
            r#"
pub struct Flag { on: std::cell::Cell<bool> }
impl Flag {
    pub fn new() -> Self { Flag { on: std::cell::Cell::new(false) } }
    #[ply::ensures(|result| *result == true || *result == false)]
    pub fn flip(&self) -> bool { self.on.set(!self.on.get()); self.on.get() }
}
"#,
            "m::Flag::flip",
        );
        let body = generate_fuzz_test(&cf, 16, &derive_seed("flip", "")).unwrap();
        assert!(body.contains("Flag::new()"), "{body}");
        assert!(
            body.contains("Flag::flip(&__ply_receiver)"),
            "a zero-arg checked call must not gain a stray trailing comma:\n{body}"
        );
    }

    // -- Defect 1 (2026-08-31, docs/reach-measurement-2.md): a method's own
    // postcondition could not mention the receiver it is called on --
    // `self` is spliced into the generated harness as a free-standing
    // expression outside any `impl` block, where the literal keyword
    // `self` means nothing (`error[E0424]`).

    /// The reported repro, verbatim: `self` and the result together.
    #[test]
    fn a_postcondition_reading_self_alongside_the_result_rewrites_self_to_the_receiver() {
        let cf = discover_receiver(
            r#"
pub struct Pair { pub a: u64 }
impl Pair {
    pub fn new(a: u64) -> Self { Pair { a } }
    #[ply::ensures(|result| *result >= self.a)]
    pub fn bumped(&self) -> u64 { self.a.saturating_add(1) }
}
"#,
            "m::Pair::bumped",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("bumped", "")).unwrap();
        assert!(
            !body.contains("self.a") && !body.contains("self . a"),
            "no bare `self` may survive into the generated harness -- it means nothing outside \
             an `impl` block:\n{body}"
        );
        assert!(
            body.contains("__ply_receiver.a") || body.contains("__ply_receiver . a"),
            "`self.a` must become a read of the receiver binding this harness already built:\n\
             {body}"
        );
    }

    /// `self` and a parameter read together in one postcondition -- only
    /// the receiver reference is rewritten, the parameter is untouched.
    #[test]
    fn a_postcondition_reading_self_and_a_parameter_rewrites_only_self() {
        let cf = discover_receiver(
            r#"
pub struct Pair { pub a: u64 }
impl Pair {
    pub fn new(a: u64) -> Self { Pair { a } }
    #[ply::ensures(|result| *result >= self.a && *result >= extra)]
    pub fn at_least(&self, extra: u64) -> u64 { self.a.saturating_add(extra) }
}
"#,
            "m::Pair::at_least",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("at_least", "")).unwrap();
        assert!(
            !body.contains("self.a") && !body.contains("self . a"),
            "no bare `self` may survive into the generated harness:\n{body}"
        );
        assert!(
            body.contains("__ply_receiver.a") || body.contains("__ply_receiver . a"),
            "`self.a` must become a read of the receiver binding:\n{body}"
        );
        assert!(
            body.contains("extra as i128"),
            "a parameter read alongside `self` must survive untouched:\n{body}"
        );
    }

    // -- seeded generation (docs/reach-measurement-2.md, "a type built from
    // text cannot be constructed from random text"): a receiver whose own
    // constructor takes a `&str`/`String` and is gated by a `requires` or a
    // fallible return grows a corpus of known-valid text (`examples:` plus
    // every value the constructor accepts during the run) and draws a mix
    // of mutations of it alongside a continuing uniform trickle, instead of
    // uniform text alone.

    /// Source 1 (design brief): the string literals a user's own
    /// `examples:` entry already passes to the constructor are extractable
    /// syntactically, with zero new vocabulary -- `examples:` entries are
    /// Rust expressions Ply already parses into assertions
    /// (`generate_example_test`).
    #[test]
    fn extracts_string_literals_passed_to_the_named_constructor() {
        let examples = vec![
            "Prerelease::new(\"beta.1\").unwrap().is_empty() == false".to_string(),
            "Prerelease::new(\"0\").unwrap().as_str() == \"0\"".to_string(),
            // A call to a different function must not contribute -- only
            // literal arguments to *this* constructor are corpus material.
            "Version::parse(\"1.2.3\").is_ok()".to_string(),
        ];
        let seeds = extract_examples_seed_strings(&examples, "Prerelease::new");
        assert_eq!(
            seeds,
            vec!["beta.1".to_string(), "0".to_string()],
            "must collect exactly the literal arguments passed to `Prerelease::new`, in order, \
             and nothing passed to an unrelated call"
        );
    }

    #[test]
    fn examples_seed_extraction_ignores_a_call_with_no_string_literal_argument() {
        let examples = vec!["Prerelease::new(some_variable).is_ok()".to_string()];
        assert_eq!(
            extract_examples_seed_strings(&examples, "Prerelease::new"),
            Vec::<String>::new(),
            "a non-literal argument is not a known-valid value Ply can embed -- nothing to \
             extract"
        );
    }

    #[test]
    fn examples_seed_extraction_skips_an_unparseable_entry_rather_than_failing() {
        let examples = vec!["Prerelease::new(".to_string()];
        assert_eq!(
            extract_examples_seed_strings(&examples, "Prerelease::new"),
            Vec::<String>::new(),
            "a malformed example is someone else's diagnostic (E0501) to report -- this \
             extractor just finds nothing in it"
        );
    }

    /// The measurement's own probe, close to verbatim: a receiver method
    /// (`is_empty`) whose receiver is built by a fallible, text-parsing
    /// constructor (`Prerelease::new`). With one `examples:`-derived seed,
    /// the generated harness must grow its corpus from it and from the
    /// constructor's own runtime accepts, not sample uniform text alone.
    #[test]
    fn a_gated_text_constructor_gets_a_seeded_strategy_when_examples_provide_a_seed() {
        let cf = discover_receiver(
            r#"
pub struct PrereleaseErr;
pub struct Prerelease { text: String }
impl Prerelease {
    pub fn new(text: &str) -> Result<Self, PrereleaseErr> {
        if text.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
            Ok(Prerelease { text: text.to_string() })
        } else {
            Err(PrereleaseErr)
        }
    }
    #[ply::ensures(|result| *result == self.text.is_empty())]
    pub fn is_empty(&self) -> bool { self.text.is_empty() }
}
"#,
            "m::Prerelease::is_empty",
        );
        let examples = vec!["Prerelease::new(\"beta.1\").is_ok()".to_string()];
        let body =
            generate_fuzz_test_with_examples(&cf, 64, &derive_seed("is_empty", ""), &examples)
                .unwrap();
        assert!(
            body.contains("__PlySeedStrategy"),
            "the ctor's own text parameter must draw from the seeded strategy, not uniform \
             text alone:\n{body}"
        );
        assert!(
            body.contains("\"beta.1\".to_string()"),
            "the example's literal argument must be embedded as an owned `String`, not a bare \
             `&str` literal -- `Vec<String>` does not accept one (`error[E0308]`, found by \
             actually compiling this exact fixture in the textseeded e2e test):\n{body}"
        );
        assert!(
            body.contains("__ply_seed_corpus.borrow_mut().push"),
            "every value the constructor accepts during the run must join the corpus too \
             (design brief, source 2):\n{body}"
        );
        assert!(
            body.contains("PLY_FUZZ_SEED_STATS|"),
            "the run must report its own provenance (examples vs. runtime-accepted counts) \
             so the verdict can carry it honestly:\n{body}"
        );
    }

    /// The other honesty condition CLAUDE.md calls out by name: a seeded
    /// run must never be indistinguishable from an unseeded one. A
    /// constructor with no `requires` and no fallible return has nothing to
    /// reject, so there is nothing to seed against -- the generated harness
    /// must come out byte-identical to what the unseeded path already
    /// produced (`generates_a_fuzz_test_for_a_scalar_fn`'s sibling case,
    /// for a receiver instead of a free fn).
    #[test]
    fn an_infallible_unconstrained_text_constructor_is_not_seeded() {
        let cf = discover_receiver(
            r#"
pub struct Label { text: String }
impl Label {
    pub fn new(text: &str) -> Self { Label { text: text.to_string() } }
    #[ply::ensures(|result| *result == self.text.len() as u32)]
    pub fn length(&self) -> u32 { self.text.len() as u32 }
}
"#,
            "m::Label::length",
        );
        let seed = derive_seed("length", "");
        let with_examples = generate_fuzz_test_with_examples(
            &cf,
            64,
            &seed,
            &["Label::new(\"x\").length()".to_string()],
        )
        .unwrap();
        let without = generate_fuzz_test(&cf, 64, &seed).unwrap();
        assert_eq!(
            with_examples, without,
            "an unconstrained constructor rejects nothing, so seeding it would be a no-op \
             disguised as a feature -- the generated harness must be unaffected either way"
        );
        assert!(
            !with_examples.contains("__PlySeedStrategy"),
            "must not seed a constructor with nothing gating it:\n{with_examples}"
        );
    }

    // -- widening past the receiver-constructor case (2026-09-01, TODO.md
    // "an example does not unblock a parameter Ply cannot build"): a *plain*
    // function's own parameter whose type Ply's ordinary codegen cannot
    // build at all (`Option<String>`, `Vec<String>` -- see `SeedableWrap`'s
    // own doc) is seeded from an `examples:` entry the same way, reusing the
    // exact corpus/mutate/trickle apparatus above rather than a second one.

    #[test]
    fn classify_seedable_wrap_recognises_option_and_vec_of_string_only() {
        assert_eq!(
            classify_seedable_wrap("Option<String>"),
            Some(SeedableWrap::OptionString)
        );
        assert_eq!(
            classify_seedable_wrap("Vec<String>"),
            Some(SeedableWrap::VecString)
        );
        assert_eq!(
            classify_seedable_wrap("Option<u32>"),
            None,
            "already buildable without seeding -- not this mechanism's job"
        );
        assert_eq!(
            classify_seedable_wrap("Widget"),
            None,
            "opaque -- no part of it is text Ply knows how to mutate"
        );
    }

    #[test]
    fn extracts_a_string_literal_from_anywhere_inside_the_named_parameter_slot() {
        let examples = vec![
            "width(Some(\"hi\".to_string())) == 2".to_string(),
            // A call to a different function must not contribute.
            "other(Some(\"nope\".to_string()))".to_string(),
        ];
        let seeds = extract_examples_seed_strings_for_param(&examples, "width", 0);
        assert_eq!(
            seeds,
            vec!["hi".to_string()],
            "must find the literal wrapped inside `Some(...)`, structure-agnostic, and nothing \
             from an unrelated call"
        );
    }

    #[test]
    fn param_seed_extraction_finds_nothing_when_no_example_calls_this_fn() {
        let examples = vec!["other(Some(\"x\".to_string()))".to_string()];
        assert_eq!(
            extract_examples_seed_strings_for_param(&examples, "width", 0),
            Vec::<String>::new()
        );
    }

    /// The worked example (TODO.md, "the measured gap"), superseded
    /// 2026-09-02 by composition: a plain fn's own `Option<String>`
    /// parameter, refused outright before this task, now builds a real
    /// strategy on its own -- **no `examples:` entry needed at all**. The
    /// old corpus-seeding workaround (`plan_param_seeding`, immediately
    /// below) existed only because `String` could not yet nest; now that it
    /// can, it never engages for this shape (its own precondition is
    /// exactly one otherwise-unbuildable parameter, and `Option<String>` is
    /// no longer one), so this test pins the real capability that replaced
    /// it rather than the workaround.
    #[test]
    fn a_plain_fns_option_string_parameter_is_fuzz_supported_via_composition() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result >= 0)]
pub fn width(label: Option<String>) -> usize { label.map(|s| s.len()).unwrap_or(0) }
"#,
            "width",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("width", "")).unwrap();
        assert!(
            body.contains("proptest::option::of("),
            "an `Option<String>` parameter must be wrapped with proptest's own `Option` \
             combinator around the ordinary curated string strategy, no seed required:\n{body}"
        );
        assert!(
            !body.contains("__PlySeedStrategy"),
            "the old corpus-seeding workaround must not engage for a shape that is now directly \
             buildable:\n{body}"
        );
    }

    /// The `Vec<String>` sibling, superseded the same way: growth by
    /// element and by length comes from `proptest::collection::vec` around
    /// the ordinary `String` strategy directly, no seed needed.
    #[test]
    fn a_plain_fns_vec_string_parameter_is_fuzz_supported_via_composition() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result >= 0)]
pub fn total_len(tags: Vec<String>) -> usize { tags.iter().map(|s| s.len()).sum() }
"#,
            "total_len",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("total_len", "")).unwrap();
        assert!(
            body.contains("proptest::collection::vec("),
            "a `Vec<String>` parameter must vary both element text and length via the ordinary \
             collection combinator, no seed required:\n{body}"
        );
        assert!(
            !body.contains("__PlySeedStrategy"),
            "the old corpus-seeding workaround must not engage for a shape that is now directly \
             buildable:\n{body}"
        );
    }

    /// The honesty condition (CLAUDE.md, "one value run 256 times is one
    /// test"): a parameter Ply cannot build *and* cannot mutate must never
    /// borrow the seeded machinery just because an example exists --
    /// `plan_param_seeding` must say `None` for it, so the caller never
    /// claims growth that cannot happen.
    #[test]
    fn an_opaque_unsupported_parameter_is_never_planned_for_seeding() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result)]
pub fn always_true(_o: Widget) -> bool { true }
"#,
            "always_true",
        );
        // Deliberately contains a string literal a broken `classify_seedable_
        // wrap` (one that answered `Some` for everything) would happily
        // treat as a seed -- so this test fails for the right reason if
        // that classifier ever stops actually gating on the shape.
        let examples = vec!["always_true(Widget::from_bytes(\"abc\")) == true".to_string()];
        assert!(
            plan_param_seeding(&cf, &examples).is_none(),
            "an opaque type has no part Ply knows how to vary -- seeding it would report a case \
             count larger than the one distinct value it actually had"
        );
    }

    // -- `examples_are_consumed`: the exact question `ply-cli`'s
    // `examples_not_run` warning asks, through the same seeding machinery
    // that actually decides it (2026-09-01) --

    /// `test` always compiles every example into a real assertion, whatever
    /// the function's own shape.
    #[test]
    fn examples_are_consumed_when_test_is_declared() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x + 1)]
pub fn increment(x: u32) -> u32 { x + 1 }
"#,
            "increment",
        );
        let examples = vec!["increment(5) == 999".to_string()];
        assert!(examples_are_consumed(&cf, &[Check::Test], &examples));
    }

    /// The real-world reproduction this whole warning exists for
    /// (`Version::parse("1.2.3").is_err()` under `checks: [fuzz(64)]`,
    /// verified by hand against `semver`): an ordinary, already-buildable
    /// parameter is never seeded, so `fuzz` alone never touches the
    /// examples at all.
    #[test]
    fn examples_are_not_consumed_by_fuzz_alone_over_an_ordinary_parameter() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x + 1)]
pub fn increment(x: u32) -> u32 { x + 1 }
"#,
            "increment",
        );
        let examples = vec!["increment(5) == 999".to_string()];
        assert!(!examples_are_consumed(&cf, &[Check::Fuzz(64)], &examples));
    }

    /// **This test's premise died on 2026-09-02 and it now records why.**
    ///
    /// It used to assert that a plain `Option<String>` parameter is seeded
    /// from `examples:`, because Ply could not build one. Making the
    /// sampling engine's shapes compose means it now builds one directly,
    /// so nothing is seeded and the examples are genuinely unconsumed --
    /// which is what this now asserts.
    ///
    /// The consequence is bigger than one test, and is recorded in TODO.md:
    /// [`classify_seedable_wrap`] accepts exactly two shapes,
    /// `Option<String>` and `Vec<String>`, and composition now builds both.
    /// **The whole plain-parameter seeding path is therefore unreachable.**
    /// The assertions below are what makes that concrete: if either shape
    /// ever stops being buildable, or a third shape is added to that
    /// classifier, this test fails and the dead path is live again.
    /// Receiver seeding -- a value built by a fallible constructor that
    /// parses text -- is untouched and still very much alive.
    #[test]
    fn composition_subsumed_the_plain_parameter_seeding_path() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result >= 0)]
pub fn width(label: Option<String>) -> usize { label.map(|s| s.len()).unwrap_or(0) }
"#,
            "width",
        );
        let examples = vec!["width(Some(\"hi\".to_string())) == 2".to_string()];
        assert!(
            !examples_are_consumed(&cf, &[Check::Fuzz(64)], &examples),
            "an `Option<String>` parameter is built directly now, so nothing seeds it \
             and the example really is unconsumed"
        );
        for src in ["Option<String>", "Vec<String>"] {
            assert!(
                classify_seedable_wrap(src).is_some(),
                "the seeding classifier is expected to still name {src}"
            );
            let ty = crate::harness::rust_type_from_source(src)
                .unwrap_or_else(|| panic!("{src} should parse as a type"));
            assert!(
                ty.is_fuzz_supported(),
                "{src} is buildable now, which is what makes the seeding path for it dead"
            );
        }
    }

    /// `textseeded`'s own shape: a receiver built by a fallible,
    /// free-form-text constructor, seeded by `fuzz` alone (no `test`
    /// declared). The warning must not fire here either.
    #[test]
    fn examples_are_consumed_by_fuzz_when_a_receiver_constructor_is_seeded() {
        let cf = discover_receiver(
            r#"
pub struct PrereleaseErr;
pub struct Prerelease { pub text: String }
impl Prerelease {
    pub fn new(text: &str) -> Result<Self, PrereleaseErr> {
        if !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
            Ok(Prerelease { text: text.to_string() })
        } else {
            Err(PrereleaseErr)
        }
    }
    #[ply::ensures(|result| *result == self.text.is_empty())]
    pub fn is_empty(&self) -> bool { self.text.is_empty() }
}
"#,
            "m::Prerelease::is_empty",
        );
        let examples = vec!["Prerelease::new(\"beta.1\").is_ok()".to_string()];
        assert!(examples_are_consumed(&cf, &[Check::Fuzz(64)], &examples));
    }

    /// Neither `test` nor `fuzz` declared at all (say, `bounded` alone):
    /// nothing here ever reads an example, whatever the function's shape.
    #[test]
    fn examples_are_not_consumed_when_neither_test_nor_fuzz_is_declared() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x + 1)]
pub fn increment(x: u32) -> u32 { x + 1 }
"#,
            "increment",
        );
        let examples = vec!["increment(5) == 999".to_string()];
        assert!(!examples_are_consumed(&cf, &[Check::Bounded(2)], &examples));
    }

    /// A receiver built through a fallible (`Result<Self, E>`) constructor
    /// -- the shape `receiverresultctor` fixed the receiver scan for --
    /// whose own postcondition also reads `self`. The two defects fixed
    /// the same day interact here: the constructor must still be found,
    /// and the postcondition must still be able to read the receiver it
    /// builds.
    #[test]
    fn a_result_returning_constructors_receiver_still_lets_its_postcondition_read_self() {
        let cf = discover_receiver(
            r#"
pub struct MeterErr;
pub struct Meter { pub n: u64 }
impl Meter {
    pub fn new(n: u64) -> Result<Self, MeterErr> {
        if n == 0 { Err(MeterErr) } else { Ok(Meter { n }) }
    }
    #[ply::ensures(|result| *result >= self.n)]
    pub fn doubled(&self) -> u64 { self.n.saturating_mul(2) }
}
"#,
            "m::Meter::doubled",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("doubled", "")).unwrap();
        assert!(
            body.contains("match Meter::new"),
            "the fallible constructor must still be recognised and called:\n{body}"
        );
        assert!(
            !body.contains("self.n") && !body.contains("self . n"),
            "no bare `self` may survive into the generated harness:\n{body}"
        );
        assert!(
            body.contains("__ply_receiver.n") || body.contains("__ply_receiver . n"),
            "`self.n` must become a read of the receiver binding, even though it was built \
             through a fallible constructor:\n{body}"
        );
    }

    // -- the sampling/proving split's second headline case (task,
    // 2026-08-27): `String`'s content and length decisions, pinned so
    // reversing either fails a test rather than silently reintroducing a
    // false alarm or an unbounded generator, the same way the float
    // NaN/infinity exclusion is pinned above.

    /// The pin for the length bound: reversing `STRING_MAX_CHARS` (or
    /// dropping the length cap from the generated strategy entirely) fails
    /// this test, because the exact bound would no longer appear in the
    /// generated source.
    #[test]
    fn string_strategy_is_length_bounded_by_the_named_constant() {
        let expr = strategy_expr(&RustType::String).unwrap();
        assert!(
            expr.contains(&format!("0..={STRING_MAX_CHARS}")),
            "the generated strategy must cap length at the one named constant, not an \
             unbounded or ad-hoc range: {expr}"
        );
        assert_eq!(
            STRING_MAX_CHARS, 32,
            "this is the disclosed bound this task chose -- changing it is fine, but must be a \
             deliberate edit to the constant, not a silent drift"
        );
    }

    /// The pin for the content decision's exclusion half: reversing either
    /// char range to include the control-character blocks
    /// (`0x00..=0x1F`/`0x7F..=0x9F`) fails this test.
    #[test]
    fn string_strategy_excludes_control_characters_by_default() {
        let expr = strategy_expr(&RustType::String).unwrap();
        // The two ranges this task chose: ASCII printable (space through
        // `~`) and everything from non-breaking space up through the top of
        // the Unicode scalar value space -- deliberately skipping both the
        // C0 control block below `0x20` and the C1 control block
        // `0x7F..=0x9F`.
        assert!(
            expr.contains("proptest::char::range('\\u{20}', '\\u{7e}')"),
            "must sample ordinary ASCII printable text: {expr}"
        );
        assert!(
            expr.contains("proptest::char::range('\\u{a0}', '\\u{10ffff}')"),
            "must also sample genuine multi-byte Unicode -- this is the type's own value \
             proposition, never excluded the way NaN is for floats: {expr}"
        );
        assert!(
            !expr.contains("any::<char>()"),
            "must not fall back to proptest's own unrestricted char Arbitrary, which would \
             reintroduce control characters: {expr}"
        );
    }

    /// The content decision's inclusion half, from the other direction:
    /// multi-byte Unicode must not be excluded the way NaN is for floats --
    /// this is the type's whole point (task brief: "the richest bug
    /// territory"). Pinned via the boundary literals the `test` tier emits.
    #[test]
    fn string_boundary_literals_include_genuine_multibyte_content() {
        let lits = boundary_literals(&RustType::String);
        assert!(!lits.is_empty());
        assert!(
            lits.iter().any(|l| l.contains("\\u{")),
            "at least one boundary literal must exercise real multi-byte Unicode content, not \
             ASCII only: {lits:?}"
        );
        assert!(
            lits.iter().any(|l| l == "String::new()"),
            "the empty string must be one of the boundary cases: {lits:?}"
        );
    }

    #[test]
    fn generates_a_fuzz_test_for_a_string_param() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == old(s).len())]
pub fn byte_len(s: String) -> usize { s.len() }
"#,
            "byte_len",
        );
        let body = generate_fuzz_test(&cf, 64, &derive_seed("byte_len", "")).unwrap();
        assert!(body.contains("fn ply_fuzz_byte_len()"));
        assert!(body.contains("proptest::char::range"));
    }

    /// `String` moves on a by-value call, exactly like `Vec<u8>` -- a
    /// postcondition reading it after the call (without `old()`) must be
    /// refused the same way, not silently compiled into a
    /// borrow-of-moved-value error.
    #[test]
    fn a_postcondition_reading_a_moved_by_value_string_is_refused_by_name() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == s.len())]
pub fn byte_len(s: String) -> usize { s.len() }
"#,
            "byte_len",
        );
        let refused = moved_param_read_in_ensures(&cf);
        assert_eq!(refused.map(|p| p.name.as_str()), Some("s"));
        let err = generate_fuzz_test(&cf, 32, &derive_seed("byte_len", "")).unwrap_err();
        assert!(err.to_string().contains("V0506"), "{err}");
    }

    /// The marker-line safety property this task's own investigation
    /// depended on (mirroring record.rs's "the record's own separator byte
    /// cannot be smuggled" proof, but for the *fuzz marker* wire format
    /// instead): a sampled string containing the marker's own separator
    /// characters (`;`, `=`, `[`, `]`) must not corrupt the marker line --
    /// `marker_display_expr`'s escaping is the mechanism, and this pins
    /// that it actually escapes every one of them, character by character.
    #[test]
    fn string_marker_display_escapes_every_wire_format_separator() {
        let expr = marker_display_expr(&RustType::String, "s");
        for (label, needle) in [
            ("backslash", "'\\\\' =>"),
            ("semicolon", "';' =>"),
            ("equals", "'=' =>"),
            ("open bracket", "'[' =>"),
            ("close bracket", "']' =>"),
            ("newline", "'\\n' =>"),
            ("carriage return", "'\\r' =>"),
        ] {
            assert!(
                expr.contains(needle),
                "the marker encoding must escape {label} -- a sampled string containing it \
                 could otherwise be mistaken for the wire format's own field/collection \
                 boundary: {expr}"
            );
        }
        // Iterates by `char`, never by byte -- a multi-byte character must
        // never be split mid-encoding.
        assert!(expr.contains(".chars()"), "{expr}");
    }
}
