//! Harness codegen: parses a contracted function's source (the §5.4a subset
//! only enough to render this slice's fixtures -- the full E0501 validator is
//! explicitly out of scope, per the M3 brief) and generates the Kani
//! `proof_for_contract` proof module, including the mandatory unwind
//! emission for `Vec`-typed parameters (§5.4b, measured in
//! docs/m3-slice-findings.md, never left to Kani's default inference).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
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
    Unsupported(String),
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
                | RustType::Bool
        )
    }

    /// A type both `kani::any()` and proptest's `any()` build directly with
    /// no construction loop: the scalars plus `char`.
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
            other => other.is_leaf() || other.is_composite_constructible(),
        }
    }

    /// The M4 gate: can the *fuzz* (proptest) codegen build this type?
    /// Strictly broader than `is_bounded_supported` -- every Kani-supported
    /// shape is fuzz-supported too, plus `Vec`/`BTreeSet` of any scalar.
    pub fn is_fuzz_supported(&self) -> bool {
        match self {
            RustType::Vec(inner) | RustType::BTreeSet(inner) => inner.is_scalar(),
            RustType::Unsupported(_) => false,
            other => other.is_bounded_supported(),
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
            // The source text as the user wrote it: for a shape Ply does not
            // model, the words they typed are the only spelling that helps.
            RustType::Unsupported(src) => src.clone(),
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
            _ => self.scalar_byte_width().is_some(),
        }
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

fn rust_type_from_syn_at(ty: &Type, aliases: &AliasMap, depth: usize) -> RustType {
    match ty {
        Type::Array(arr) => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(n),
                ..
            }) = &arr.len
            else {
                return RustType::Unsupported(ty.to_token_stream().to_string());
            };
            let Ok(n) = n.base10_parse::<u32>() else {
                return RustType::Unsupported(ty.to_token_stream().to_string());
            };
            let elem = rust_type_from_syn_at(&arr.elem, aliases, depth);
            if elem.is_leaf() || elem.is_composite_constructible() {
                RustType::Array(Box::new(elem), n)
            } else {
                RustType::Unsupported(ty.to_token_stream().to_string())
            }
        }
        Type::Path(tp) => {
            let Some(seg) = tp.path.segments.last() else {
                return RustType::Unsupported(ty.to_token_stream().to_string());
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
                        if inner.is_leaf() || inner.is_composite_constructible() {
                            return RustType::Option(Box::new(inner));
                        }
                    }
                    RustType::Unsupported(ty.to_token_stream().to_string())
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
                            let usable =
                                |r: &RustType| r.is_leaf() || r.is_composite_constructible();
                            if usable(&ok) && usable(&err) {
                                return RustType::Result(Box::new(ok), Box::new(err));
                            }
                        }
                    }
                    RustType::Unsupported(ty.to_token_stream().to_string())
                }
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
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
                        && let Some(syn::GenericArgument::Type(inner_ty)) = ab.args.first()
                    {
                        if let Type::Path(inner) = inner_ty
                            && inner.path.is_ident("u8")
                        {
                            return RustType::VecU8;
                        }
                        let inner = rust_type_from_syn_at(inner_ty, aliases, depth);
                        if inner.is_scalar() {
                            return RustType::Vec(Box::new(inner));
                        }
                    }
                    RustType::Unsupported(ty.to_token_stream().to_string())
                }
                // Fuzz-only (§5.4b measured exclusion): proptest has no
                // trouble generating a BTreeSet of scalars; Kani does, past
                // one element, at any bound.
                "BTreeSet" => {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments
                        && let Some(syn::GenericArgument::Type(inner_ty)) = ab.args.first()
                    {
                        let inner = rust_type_from_syn_at(inner_ty, aliases, depth);
                        if inner.is_scalar() {
                            return RustType::BTreeSet(Box::new(inner));
                        }
                    }
                    RustType::Unsupported(ty.to_token_stream().to_string())
                }
                _ => RustType::Unsupported(ty.to_token_stream().to_string()),
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
        other => RustType::Unsupported(other.to_token_stream().to_string()),
    }
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
            build_contract_fn(&found.item, &alias_map(&found.file), &found.canonical)
                .map_err(AnchorError::Shape)
        }
        crate::callgraph::Resolution::Opaque(reason) => Err(AnchorError::Unreadable(reason)),
        crate::callgraph::Resolution::NotFound => Err(AnchorError::NotFound),
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
fn tidy_contract_text(s: &str) -> String {
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
pub fn build_contract_fn(f: &ItemFn, aliases: &AliasMap, path: &str) -> Result<ContractFn> {
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

    Ok(ContractFn {
        name,
        path: path.to_string(),
        params,
        requires,
        ensures,
        calls: crate::callgraph::call_sites(f),
        source: f.to_token_stream().to_string(),
    })
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
        !matches!(
            self.kind,
            StubKind::Contracted {
                bound: Some(_),
                ..
            }
        )
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
        format!(
            "#[cfg(kani)]\n\
             #[allow(dead_code, unused_variables)]\n\
             #[kani::proof_for_contract({path})]\n\
             fn {name}() {{\n\
             {lets}\
             \x20\x20\x20\x20{path}({args});\n\
             }}\n",
            path = self.callee_path,
            name = format!("ply_verified_exists_{}", self.callee_path.replace("::", "_")),
            lets = lets,
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

    let promise = crate::promise::plan(stubs);
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

    #[test]
    fn an_array_of_a_shape_kani_cannot_build_is_still_unsupported() {
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
        assert!(
            matches!(cf.params[0].ty, RustType::Unsupported(_)),
            "widening the fragment must not widen it past what the engines build: {:?}",
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
}
