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
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    match names.len() {
        // A zero-parameter fn (a receiverless constructor like
        // `FakeClock::new()`) has nothing for proptest to generate --
        // `combined_strategy_expr_for` pairs this with `Just(())`, so the
        // pattern that destructures it is `_`, never the empty-tuple
        // pattern `()` the old `else` branch produced (which only ever
        // matched a *value* of `()`, and the strategy below did not build
        // one -- see that fn's own doc for the compile error this caused).
        0 => "_".to_string(),
        1 => names[0].to_string(),
        _ => format!("({})", names.join(", ")),
    }
}

fn value_pattern(cf: &ContractFn) -> String {
    value_pattern_for(&cf.params)
}

fn combined_strategy_expr_for(params: &[Param]) -> Result<String> {
    let exprs: Result<Vec<String>> = params.iter().map(|p| strategy_expr(&p.ty)).collect();
    let exprs = exprs?;
    Ok(match exprs.len() {
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
        1 => exprs[0].clone(),
        _ => format!("({})", exprs.join(", ")),
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
        RustType::VecU8 | RustType::Vec(_) | RustType::BTreeSet(_) => true,
        RustType::Option(inner) => moves_on_by_value_call(inner),
        RustType::Result(ok, err) => moves_on_by_value_call(ok) || moves_on_by_value_call(err),
        RustType::Array(inner, _) => moves_on_by_value_call(inner),
        RustType::Unsupported(_) => false,
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
/// (already-computed) pattern and strategy -- reused verbatim for every
/// operation in the pool, since [`harness::Operation::params_match`]
/// guarantees every pooled operation shares the checked method's exact
/// parameter shape. This is why the codegen never builds a mixed-shape
/// step: there is only ever one argument shape to generate, no matter which
/// operation a given step calls.
fn receiver_pattern_and_strategy(
    plan: &harness::ReceiverPlan,
    target_pattern: &str,
    target_strategy: &str,
) -> Result<(String, String)> {
    let ctor_pattern = value_pattern_for(&plan.ctor_params);
    let ctor_strategy = combined_strategy_expr_for(&plan.ctor_params)?;
    let num_ops = plan.operations.len();
    let seq_strategy = format!(
        "proptest::collection::vec((0u8..{num_ops}u8, {target_strategy}), 0..={max}usize)",
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
/// arguments (the same shape as the checked method's own, per
/// `receiver_pattern_and_strategy`'s doc).
fn receiver_preamble(plan: &harness::ReceiverPlan, target_pattern: &str) -> String {
    let ctor_call = harness::last_two_segments(&plan.constructor);
    let ctor_args = call_args_for(&plan.ctor_params).join(", ");
    let mut body = format!("let __ply_receiver = {ctor_call}({ctor_args});\n            ");
    body.push_str(&format!(
        "for (__ply_op_choice, {target_pattern}) in __ply_seq {{\n"
    ));
    body.push_str("                match __ply_op_choice {\n");
    for (i, op) in plan.operations.iter().enumerate() {
        let call = harness::last_two_segments(&op.call_path);
        let op_args = call_args_for(&op.params).join(", ");
        let full_args = if op_args.is_empty() {
            "&__ply_receiver".to_string()
        } else {
            format!("&__ply_receiver, {op_args}")
        };
        body.push_str(&format!(
            "                    {i} => {{ let _ = {call}({full_args}); }}\n"
        ));
    }
    body.push_str(
        "                    _ => unreachable!(\"__ply_op_choice is generated in 0..num_ops\"),\n",
    );
    body.push_str("                }\n            }\n            ");
    body
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
            (p, s, receiver_preamble(plan, &target_pattern))
        }
        None => (
            target_pattern.clone(),
            target_strategy.clone(),
            String::new(),
        ),
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

    // `old(expr)` -- the value `expr` had on entry -- is read into a
    // binding of its own before the call, which is the only way a harness
    // built out of ordinary Rust can honour it (§5.4a).
    let (checked_body, entry_values) =
        crate::contract_rt::lift_entry_values(&cf.ensures.as_ref().unwrap().0.body);
    let entry_lets = crate::contract_rt::entry_value_lets(&entry_values, &" ".repeat(12));
    let widened = crate::contract_rt::widen(&checked_body).to_string();

    let marker_fields: Vec<String> = cf
        .params
        .iter()
        .map(|p| format!("{}={}", p.name, marker_display_expr(&p.ty, &p.name)))
        .collect();
    // Each field is itself an *expression*; join them at runtime, not here,
    // since a Vec/BTreeSet field is a block expression, not a literal.
    let mut marker_build = String::from("let mut __ply_marker = String::new();\n");
    marker_build.push_str(&format!(
        "            __ply_marker.push_str(\"PLY_FUZZED_CEX|{label}|\");\n"
    ));
    for (i, field) in marker_fields.iter().enumerate() {
        let (name, value_expr) = field.split_once('=').expect("field has name=expr shape");
        if i > 0 {
            marker_build.push_str("            __ply_marker.push(';');\n");
        }
        marker_build.push_str(&format!(
            "            __ply_marker.push_str(&format!(\"{name}={{}}\", {value_expr}));\n"
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
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20{requires_check}{receiver_preamble_text}{entry_lets}let __ply_call_result = {fname}({args});\n\
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
                format!("vec![]"),
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
        // Never reached: return-only shapes, never a parameter's.
        RustType::SelfType | RustType::Unit | RustType::Unsupported(_) => vec![],
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
        "#[cfg(test)]\nmod {module_ident}_harness {{\n    #[allow(unused_imports)]\n    use {target_crate_ident}::{import_path};\n\n"
    );
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
}
