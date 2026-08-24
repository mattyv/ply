//! Harness codegen: parses a contracted function's source (the §5.4a subset
//! only enough to render this slice's fixtures -- the full E0501 validator is
//! explicitly out of scope, per the M3 brief) and generates the Kani
//! `proof_for_contract` proof module, including the mandatory unwind
//! emission for `Vec`-typed parameters (§5.4b, measured in
//! docs/m3-slice-findings.md, never left to Kani's default inference).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use quote::ToTokens;
use syn::{Expr, ExprClosure, FnArg, ItemFn, Pat, Type};

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    Unsupported(String),
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
                | RustType::Bool
        )
    }

    /// The narrower gate: can Ply's *Kani* codegen build this type at all?
    /// (Renamed from the M3 slice's `is_supported` now that a second,
    /// broader gate -- `is_fuzz_supported` -- exists; every M3 call site is
    /// updated to this name, behaviour unchanged for every type M3 knew
    /// about.)
    pub fn is_bounded_supported(&self) -> bool {
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
                | RustType::Bool
                | RustType::VecU8
        )
    }

    /// The M4 gate: can the *fuzz* (proptest) codegen build this type?
    /// Strictly broader than `is_bounded_supported` -- every Kani-supported
    /// shape is fuzz-supported too, plus `Vec`/`BTreeSet` of any scalar.
    pub fn is_fuzz_supported(&self) -> bool {
        match self {
            RustType::Vec(inner) | RustType::BTreeSet(inner) => inner.is_scalar(),
            RustType::Unsupported(_) => false,
            other => other.is_bounded_supported() || other.is_scalar(),
        }
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
            RustType::Bool => "bool",
            _ => return None,
        })
    }

    /// Byte width Kani's concrete-playback encodes this scalar as
    /// (little-endian on the pinned toolchain's target -- measured, see
    /// docs/m3-slice-findings.md).
    pub fn scalar_byte_width(&self) -> Option<usize> {
        match self {
            RustType::U8 | RustType::I8 | RustType::Bool => Some(1),
            RustType::U16 | RustType::I16 => Some(2),
            RustType::U32 | RustType::I32 => Some(4),
            RustType::U64 | RustType::I64 => Some(8),
            RustType::VecU8
            | RustType::Vec(_)
            | RustType::BTreeSet(_)
            | RustType::Unsupported(_) => None,
        }
    }
}

fn rust_type_from_syn(ty: &Type) -> RustType {
    match ty {
        Type::Path(tp) => {
            let Some(seg) = tp.path.segments.last() else {
                return RustType::Unsupported(ty.to_token_stream().to_string());
            };
            match seg.ident.to_string().as_str() {
                "u8" => RustType::U8,
                "u16" => RustType::U16,
                "u32" => RustType::U32,
                "u64" => RustType::U64,
                "i8" => RustType::I8,
                "i16" => RustType::I16,
                "i32" => RustType::I32,
                "i64" => RustType::I64,
                "bool" => RustType::Bool,
                "Vec" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        if let Some(syn::GenericArgument::Type(inner_ty)) = ab.args.first() {
                            if let Type::Path(inner) = inner_ty {
                                if inner.path.is_ident("u8") {
                                    return RustType::VecU8;
                                }
                            }
                            let inner = rust_type_from_syn(inner_ty);
                            if inner.is_scalar() {
                                return RustType::Vec(Box::new(inner));
                            }
                        }
                    }
                    RustType::Unsupported(ty.to_token_stream().to_string())
                }
                // Fuzz-only (§5.4b measured exclusion): proptest has no
                // trouble generating a BTreeSet of scalars; Kani does, past
                // one element, at any bound.
                "BTreeSet" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        if let Some(syn::GenericArgument::Type(inner_ty)) = ab.args.first() {
                            let inner = rust_type_from_syn(inner_ty);
                            if inner.is_scalar() {
                                return RustType::BTreeSet(Box::new(inner));
                            }
                        }
                    }
                    RustType::Unsupported(ty.to_token_stream().to_string())
                }
                _ => RustType::Unsupported(ty.to_token_stream().to_string()),
            }
        }
        Type::Reference(r) => rust_type_from_syn(&r.elem),
        other => RustType::Unsupported(other.to_token_stream().to_string()),
    }
}

#[derive(Debug, Clone)]
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
    pub name: String,
    pub params: Vec<Param>,
    /// `#[ply::requires(expr)]`, if present: the raw boolean expression.
    pub requires: Option<(Expr, String)>,
    /// `#[ply::ensures(|result| expr)]`, if present: the closure (its single
    /// parameter is conventionally named `result`, matching Kani's own
    /// `kani::ensures` shape) plus its source text for diagnostics.
    pub ensures: Option<(ExprClosure, String)>,
}

impl ContractFn {
    /// Can Ply's Kani codegen build this fn's harness at all? (§5.4b gate.)
    pub fn is_bounded_supported(&self) -> bool {
        self.params.iter().all(|p| p.ty.is_bounded_supported())
    }

    /// Can Ply's proptest codegen build this fn's harness? (M4 gate --
    /// strictly broader, see `RustType::is_fuzz_supported`.)
    pub fn is_fuzz_supported(&self) -> bool {
        self.params.iter().all(|p| p.ty.is_fuzz_supported())
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
}

/// Parses `src_path` and returns the contracted function named `fn_name`.
/// Only top-level free functions are supported in this slice (no `impl`
/// methods, no nested modules) -- the full extractor (§4's `extract/`
/// module) is out of scope.
pub fn discover_fn(src_path: &Path, fn_name: &str) -> Result<ContractFn> {
    let src = std::fs::read_to_string(src_path)
        .with_context(|| format!("reading source at {}", src_path.display()))?;
    let file = syn::parse_file(&src)
        .with_context(|| format!("parsing source at {}", src_path.display()))?;
    for item in &file.items {
        if let syn::Item::Fn(f) = item {
            if f.sig.ident == fn_name {
                return build_contract_fn(f);
            }
        }
    }
    bail!(
        "E0301: could not find fn `{fn_name}` in {} (unresolvable anchor)",
        src_path.display()
    )
}

/// `quote`'s `TokenStream::to_string()` inserts a space between every token
/// (`|result|` becomes `| result |`), which is faithful but fails the
/// newbie-bar bar for text a user reads in a diagnostic or a generated
/// test's doc comment. This is a deliberately narrow cosmetic cleanup for
/// the closure-pipe and leading-deref shapes this slice's own contracts
/// use -- not a general Rust pretty-printer.
fn tidy_contract_text(s: &str) -> String {
    s.replace("| ", "|").replace(" |", "|").replace("* ", "*").replace(" . ", ".").replace(" ()", "()")
}

fn build_contract_fn(f: &ItemFn) -> Result<ContractFn> {
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
        let (by_ref, inner_ty) = match &*pt.ty {
            Type::Reference(r) => (true, r.elem.as_ref()),
            other => (false, other),
        };
        params.push(Param { name: pname, ty: rust_type_from_syn(inner_ty), by_ref });
    }

    let mut requires = None;
    let mut ensures = None;
    for attr in &f.attrs {
        let segs: Vec<String> =
            attr.path().segments.iter().map(|s| s.ident.to_string()).collect();
        if segs == ["ply", "requires"] {
            let expr: Expr = attr.parse_args().context("E0501: could not parse #[ply::requires] as an expression")?;
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

    Ok(ContractFn { name, params, requires, ensures })
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
}

/// Generates the `#[kani::proof_for_contract]` harness for `cf`, sized by
/// `bound_k` (the declared `bounded(k)` -- also used as the Vec length bound
/// when the function has a `Vec<u8>` parameter). Emits `#[kani::unwind(k+1)]`
/// whenever a Vec parameter is present -- §5.4b's mandatory annotation,
/// measured (not inferred) for exactly this manual-indexed-loop-consumption
/// shape in docs/m3-slice-findings.md. Without it, Kani's default unwind
/// inference times out at every length, including 1.
pub fn generate_proof_module(cf: &ContractFn, bound_k: u32) -> Result<GeneratedHarness> {
    if !cf.is_bounded_supported() {
        let bad: Vec<String> = cf
            .params
            .iter()
            .filter(|p| !p.ty.is_bounded_supported())
            .map(|p| format!("{}: {:?}", p.name, p.ty))
            .collect();
        bail!("V0505: unsupported parameter type(s) for `{}`: {}", cf.name, bad.join(", "));
    }

    let mut lets = String::new();
    let mut call_args = Vec::new();
    let has_vec = cf.has_vec_param();

    for p in &cf.params {
        match &p.ty {
            RustType::VecU8 => {
                lets.push_str(&format!(
                    "    let {name} = kani::vec::any_vec::<u8, {n}>();\n",
                    name = p.name,
                    n = bound_k
                ));
            }
            other => {
                let ty_name = other.scalar_rust_name().expect("checked supported above");
                lets.push_str(&format!(
                    "    let {name}: {ty} = kani::any();\n",
                    name = p.name,
                    ty = ty_name
                ));
            }
        }
        call_args.push(if p.by_ref { format!("&{}", p.name) } else { p.name.clone() });
    }

    let unwind = if has_vec { Some(bound_k + 1) } else { None };
    let unwind_attr = unwind
        .map(|n| format!("#[kani::unwind({n})]\n"))
        .unwrap_or_default();

    let proof_fn_name = format!("ply_proof_{}", cf.name);
    let module_source = format!(
        "//! Generated by Ply -- do not edit. Kani proof harness for `{fname}`\n\
         //! (check bounded({k})). See The-Ply-Spec.md D2 and §5.4b.\n\
         #[cfg(kani)]\n\
         use super::*;\n\n\
         #[cfg(kani)]\n\
         #[kani::proof_for_contract({fname})]\n\
         {unwind_attr}\
         fn {proof_fn_name}() {{\n\
         {lets}\
         \x20\x20\x20\x20{fname}({args});\n\
         }}\n",
        fname = cf.name,
        k = bound_k,
        unwind_attr = unwind_attr,
        proof_fn_name = proof_fn_name,
        lets = lets,
        args = call_args.join(", "),
    );

    Ok(GeneratedHarness {
        module_source,
        proof_fn_path: format!("ply_generated::{proof_fn_name}"),
        unwind,
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
    write_generated_file(crate_src_dir, lib_path, "ply_generated_cex", test_module_source)
}

fn write_generated_file(
    crate_src_dir: &Path,
    lib_path: &Path,
    file_stem: &str,
    source: &str,
) -> Result<PathBuf> {
    let out_path = crate_src_dir.join(format!("{file_stem}.rs"));
    std::fs::write(&out_path, source)
        .with_context(|| format!("writing {}", out_path.display()))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Contract text is quoted verbatim into diagnostics and into the
    /// rendered cex test's own failure message, so it has to read like the
    /// line the user wrote. Token-stream text spaces out every token, and
    /// the tidier only knew about `|` and `*` -- any contract calling a
    /// method came out as `xs . len () as u32` (2026-08-24 M4 review, D7's
    /// side observation, seen for real on the `BTreeSet` witness path).
    #[test]
    fn contract_text_reads_like_the_line_the_user_wrote_even_with_method_calls() {
        assert_eq!(tidy_contract_text("| result | * result == xs . len () as u32"), "|result|*result == xs.len() as u32");
        // The M3 shapes stay exactly as they were.
        assert_eq!(tidy_contract_text("| result | * result >= lo"), "|result|*result >= lo");
    }

    fn write_src(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("lib.rs");
        std::fs::write(&path, content).unwrap();
        path
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
        assert_eq!(cf.params[0].ty, RustType::VecU8, "M3's VecU8 shape must not regress to Vec(U8)");
        assert!(cf.is_bounded_supported());
        assert!(cf.is_fuzz_supported());
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
        let harness_out = generate_proof_module(&cf, 2).unwrap();
        assert!(harness_out.unwind.is_none(), "scalar-only fn must not get an unwind annotation");
        assert!(harness_out.module_source.contains("kani::any()"));
        assert!(harness_out.module_source.contains("#[kani::proof_for_contract(clamp)]"));
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
        let harness_out = generate_proof_module(&cf, 8).unwrap();
        assert_eq!(harness_out.unwind, Some(9), "measured bound for N=8 is N+1=9 (see m3-slice-findings.md)");
        assert!(harness_out.module_source.contains("#[kani::unwind(9)]"));
        assert!(harness_out.module_source.contains("kani::vec::any_vec::<u8, 8>()"));
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
        assert_eq!(after_first, after_second, "mod declaration must be inserted exactly once");
        assert_eq!(
            std::fs::read_to_string(src_dir.join("ply_generated.rs")).unwrap(),
            "// two\n",
            "the generated file's content still updates on rerun"
        );
    }
}
