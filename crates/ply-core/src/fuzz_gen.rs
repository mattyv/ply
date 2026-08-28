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
        // `char`, `Option<T>`, `Result<T, E>` and `[T; N]` all have
        // proptest `Arbitrary` impls, so their strategy is just `any()`.
        // No small-magnitude bias here: the interesting values of these
        // shapes are the variants and the element pattern, not the size.
        RustType::Char | RustType::Option(_) | RustType::Result(..) | RustType::Array(..) => {
            let name = ty.rust_name().expect("composite has a rust name");
            format!("proptest::prelude::any::<{name}>()")
        }
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
        // Never reached either, for a different reason: a struct/enum
        // parameter is never handed to `strategy_expr` directly -- it has
        // no single scalar `Strategy` of its own. `plan_for_param`/
        // `build_user_value_stmt` (below) draw a *leaf* strategy per
        // constructor argument or field instead, and call `strategy_expr`
        // only on each of those (always an ordinary type by the time it
        // gets there -- `resolve_user_type` never resolves a leaf to
        // another unresolved shape). Reaching this arm would mean some
        // caller skipped that machinery, which is a Ply bug, not a user
        // error.
        RustType::UserTypeCtor(_) | RustType::UserTypeFields(_) => {
            bail!(
                "`{}` has no strategy of its own -- it is built from its own constructor's or \
                 fields' leaf strategies, never sampled directly; this is a Ply bug, not a user \
                 error",
                ty.display_name()
            )
        }
    })
}

/// The Rust expression text that turns a bound variable `var: ty` into a
/// `Display`-able value for the `PLY_FUZZED_CEX` marker line -- a plain
/// variable reference for scalars (their own `Display` impl), or an inline
/// block that joins a collection into `[a,b,c]` text (no spaces, so the
/// decoder's split-on-comma is exact) for `Vec`/`BTreeSet`.
fn marker_display_expr(ty: &RustType, var: &str) -> String {
    match ty {
        // No `Display` impl for any of these; `Debug` is what a reader of
        // the diagnostic wants to see anyway (`Some(3)`, `'x'`, `[1, 2]`).
        RustType::Char | RustType::Option(_) | RustType::Result(..) | RustType::Array(..) => {
            format!("format!(\"{{:?}}\", {var})")
        }
        RustType::Vec(_) | RustType::VecU8 | RustType::BTreeSet(_) => format!(
            "{{ let mut __ply_s = String::from(\"[\"); \
             for (__ply_i, __ply_e) in {var}.iter().enumerate() {{ \
             if __ply_i > 0 {{ __ply_s.push(','); }} \
             __ply_s.push_str(&__ply_e.to_string()); }} \
             __ply_s.push(']'); __ply_s }}"
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

/// [`value_pattern_for`] with every name prefixed for operation `i` --
/// used for every pooled operation's own slot in the sequence's per-step
/// tuple pattern except operation zero's.
fn value_pattern_for_prefixed(params: &[Param], i: usize) -> String {
    let prefix = op_prefix(i);
    let names: Vec<String> = params
        .iter()
        .map(|p| format!("{prefix}{}", p.name))
        .collect();
    match names.len() {
        0 => "_".to_string(),
        1 => names[0].clone(),
        _ => format!("({})", names.join(", ")),
    }
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
        _ => Ok(ParamPlan {
            pattern: p.name.clone(),
            strategy: strategy_expr(&p.ty)?,
            preamble: String::new(),
        }),
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
        RustType::Unsupported(_) => false,
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
/// Every *other* pooled operation gets its own strategy and its own
/// (prefixed) pattern (2026-08-27, docs/review-caveats.md N3): the pool is
/// no longer restricted to the checked method's own parameter shape, so a
/// mixed-shape step needs its own slot per operation rather than one shared
/// one.
fn receiver_pattern_and_strategy(
    plan: &harness::ReceiverPlan,
    target_pattern: &str,
    target_strategy: &str,
) -> Result<(String, String)> {
    let ctor_pattern = value_pattern_for(&plan.ctor_params);
    let ctor_strategy = combined_strategy_expr_for(&plan.ctor_params)?;
    let num_ops = plan.operations.len();
    let mut step_strategies = vec![target_strategy.to_string()];
    for op in plan.operations.iter().skip(1) {
        step_strategies.push(combined_strategy_expr_for(&op.params)?);
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
    body.push_str(&format!(
        "let {mut_kw}__ply_receiver = {ctor_call}({ctor_args});\n            "
    ));

    let step_pattern = plan
        .operations
        .iter()
        .enumerate()
        .map(|(i, op)| {
            if i == 0 {
                target_pattern.to_string()
            } else {
                value_pattern_for_prefixed(&op.params, i)
            }
        })
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
        let call_stmt = format!("let _ = {call}({full_args});");
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
/// the example/direct-case tests.
pub fn generate_fuzz_test(cf: &ContractFn, cases: u32, seed: &[u8; 32]) -> Result<String> {
    let Some((_closure, _)) = &cf.ensures else {
        bail!(
            "fuzz check requires an #[ply::ensures] clause on `{}` to check against",
            cf.name
        );
    };
    if !cf.is_fuzz_supported() {
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

    let target_pattern = value_pattern(cf);
    let target_strategy = combined_strategy_expr(cf)?;
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
    let (pattern, strategy, receiver_preamble_text) = match &cf.receiver {
        Some(plan) => {
            let (p, s) = receiver_pattern_and_strategy(plan, &target_pattern, &target_strategy)?;
            (p, s, receiver_preamble(cf, plan, &target_pattern)?)
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
    let params_preamble_text = params_preamble(&cf.params)?;
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

    // `old(expr)` -- the value `expr` had on entry -- is read into a
    // binding of its own before the call, which is the only way a harness
    // built out of ordinary Rust can honour it (§5.4a).
    let (checked_body, entry_values) =
        crate::contract_rt::lift_entry_values(&cf.ensures.as_ref().unwrap().0.body);
    let entry_lets = crate::contract_rt::entry_value_lets(&entry_values, &" ".repeat(12));
    let widened = crate::contract_rt::widen(&checked_body).to_string();

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
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_strategy = {strategy};\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let __ply_outcome = __ply_runner.run(&__ply_strategy, |{pattern}| {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20__ply_total.set(__ply_total.get() + 1);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20{params_preamble_text}{requires_check}{receiver_preamble_text}{entry_lets}{marker_precompute}let __ply_call_result = {fname}({args});\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let result = &__ply_call_result;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let __ply_ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {widened}));\n\
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
pub fn generate_example_test(fn_name: &str, index: u32, example_src: &str) -> Result<String> {
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
    Ok(format!(
        "    #[test]\n\
         \x20\x20\x20\x20fn ply_example_{fn_name}_{index:02}() {{\n\
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
        RustType::Vec(inner) => {
            let n = inner.scalar_rust_name().unwrap_or("i64");
            vec![
                "vec![]".to_string(),
                format!("vec![0{n}]"),
                format!("vec![1{n}; 8]"),
            ]
        }
        RustType::BTreeSet(inner) => {
            let n = inner.scalar_rust_name().unwrap_or("i64");
            vec![
                "std::collections::BTreeSet::new()".to_string(),
                format!("std::collections::BTreeSet::from([0{n}])"),
                format!("std::collections::BTreeSet::from([0{n}, 1{n}, 2{n}])"),
            ]
        }
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
    let widened = crate::contract_rt::widen(&checked_body).to_string();
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
/// field/variant's own type -- deduplicated, in first-seen order.
/// Struct/enum parameters (2026-08-27): `call_expr()`'s own import only
/// brings the checked function (or its enclosing type, for a method) into
/// scope, never a *parameter's* type -- `wrap_fn_harness_module` needs one
/// more `use` per such type, or the generated harness fails to compile with
/// "cannot find struct/enum `X` in this scope" the moment it names one in a
/// constructor call or a field/variant literal.
fn extra_type_imports(cf: &ContractFn) -> Vec<String> {
    fn walk(ty: &RustType, out: &mut Vec<String>) {
        match ty {
            RustType::UserTypeCtor(plan) => {
                if !out.iter().any(|n| n == &plan.import_path) {
                    out.push(plan.import_path.clone());
                }
                for p in &plan.ctor_params {
                    walk(&p.ty, out);
                }
            }
            RustType::UserTypeFields(plan) => {
                if !out.iter().any(|n| n == &plan.import_path) {
                    out.push(plan.import_path.clone());
                }
                match &plan.shape {
                    harness::UserTypeShape::Struct(fields) => {
                        for f in fields {
                            walk(&f.ty, out);
                        }
                    }
                    harness::UserTypeShape::Enum(variants) => {
                        for (_, fields) in variants {
                            for f in fields {
                                walk(&f.ty, out);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for p in &cf.params {
        walk(&p.ty, &mut out);
    }
    if let Some(plan) = &cf.receiver {
        for p in &plan.ctor_params {
            walk(&p.ty, &mut out);
        }
    }
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
        "#[cfg(test)]\nmod {module_ident}_harness {{\n    #[allow(unused_imports)]\n    use {target_crate_ident}::{import_path};\n"
    );
    // Struct/enum parameters (2026-08-27): one `use` per type Ply itself
    // constructs -- assumed to sit at the target crate's own root, same as
    // every other bare struct/enum name this scan resolves
    // (`scan_crate_type_locations` indexes by bare name, not module path).
    for extra in extra_type_imports(cf) {
        out.push_str(&format!(
            "    #[allow(unused_imports)]\n    use {target_crate_ident}::{extra};\n"
        ));
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
        let body = generate_example_test("clamp", 1, "clamp(150) == 100").unwrap();
        assert!(body.contains("fn ply_example_clamp_01()"));
        assert!(body.contains("clamp (150) == 100") || body.contains("clamp(150) == 100"));
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
        let body = generate_example_test("greet", 1, r#"greet(0) == "zero""#).unwrap();
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
        harness::discover_method_with_receiver(dir.path(), fn_path).unwrap()
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
