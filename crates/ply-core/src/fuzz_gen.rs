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

use anyhow::{Result, bail};
use quote::ToTokens;
use syn::Expr;

use crate::harness::{ContractFn, RustType};

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
        RustType::U8 | RustType::U16 | RustType::U32 | RustType::U64 => {
            let name = ty.scalar_rust_name().expect("scalar");
            format!(
                "proptest::prop_oneof![3 => 0{name}..=16{name}, 1 => proptest::prelude::any::<{name}>()]"
            )
        }
        RustType::I8 | RustType::I16 | RustType::I32 | RustType::I64 => {
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
        RustType::Unsupported(t) => {
            bail!("cannot build a fuzz strategy for unsupported type `{t}`")
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
        _ => var.to_string(),
    }
}

fn param_names(cf: &ContractFn) -> Vec<&str> {
    cf.params.iter().map(|p| p.name.as_str()).collect()
}

/// The closure/loop-bound pattern proptest's `TestRunner::run` needs to
/// destructure the strategy's produced value: a bare name for one param, a
/// parenthesized tuple pattern for more than one (matching the tuple
/// strategy `combined_strategy_expr` builds for that case).
fn value_pattern(cf: &ContractFn) -> String {
    let names = param_names(cf);
    if names.len() == 1 {
        names[0].to_string()
    } else {
        format!("({})", names.join(", "))
    }
}

fn combined_strategy_expr(cf: &ContractFn) -> Result<String> {
    let exprs: Result<Vec<String>> = cf.params.iter().map(|p| strategy_expr(&p.ty)).collect();
    let exprs = exprs?;
    Ok(if exprs.len() == 1 {
        exprs[0].clone()
    } else {
        format!("({})", exprs.join(", "))
    })
}

fn call_args(cf: &ContractFn) -> Vec<String> {
    cf.params
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

    let pattern = value_pattern(cf);
    let seed_literal = format!(
        "[{}]",
        seed.iter()
            .map(|b| format!("{b}u8"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let seed_hex = seed_hex(seed);
    let strategy = combined_strategy_expr(cf)?;
    let args = call_args(cf).join(", ");
    // Two spellings, deliberately. `fname` is the bare identifier the
    // harness module imports and therefore calls; `label` is where the
    // function lives, which is what a reader of the output needs to see.
    // They differ only for a function inside a module.
    let fname = &cf.name;
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
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20{requires_check}{entry_lets}let __ply_call_result = {fname}({args});\n\
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
        RustType::U8 | RustType::U16 | RustType::U32 | RustType::U64 => {
            let n = ty.scalar_rust_name().unwrap();
            vec![format!("0{n}"), format!("1{n}"), format!("{n}::MAX")]
        }
        RustType::I8 | RustType::I16 | RustType::I32 | RustType::I64 => {
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
        RustType::Unsupported(_) => vec![],
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
    let literal_sets: Vec<Vec<String>> =
        cf.params.iter().map(|p| boundary_literals(&p.ty)).collect();
    if literal_sets.iter().any(|s| s.is_empty()) {
        return String::new();
    }
    let n_cases = literal_sets.iter().map(|s| s.len()).max().unwrap_or(0);
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
            fname = cf.name,
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
    let fn_path = &cf.path;
    let mut out = format!(
        "#[cfg(test)]\nmod {module_ident}_harness {{\n    #[allow(unused_imports)]\n    use {target_crate_ident}::{fn_path};\n\n"
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
}
