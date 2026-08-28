//! Measures Ply's real type-mapping coverage against the Flowgate rate
//! limiter (`tests/fixtures/ratelimiter/`), a crate written by someone told
//! nothing about Ply -- the yardstick named in the 2026-08-26 task ("add
//! usize/isize/NonZero family/Duration").
//!
//! **Why this measures the type mapping directly rather than running
//! `cargo ply verify`'s real claim pipeline end to end**: every function in
//! this crate is a method or an associated function (`self`/`Self`
//! receivers, `impl` blocks), and `ply_core::callgraph::Resolver` --
//! confirmed by reading it, not assumed -- indexes only free functions
//! (`syn::Item::Fn`) reachable from the crate root by module path. It has no
//! `syn::Item::Impl` arm at all. So a `ply.yaml` claim naming any method by
//! its natural path (`TokenBucket::check_n`) resolves to nothing today
//! (`E0301`, "could not find a function"), for a reason that has nothing to
//! do with which types its parameters use -- confirmed for real against this
//! crate's own `measure.ply.yaml`: `cargo ply check` there resolves exactly
//! 1 of 39 claimed items (the one free fn, `internal::refill_and_debit`,
//! itself excluded from this measurement's supported-type question because
//! it is generic over `C: Clock` and takes `&mut` parameters -- unrelated to
//! this task and unaffected by it). That is a real, separate, out-of-scope
//! gap (Ply does not yet claim methods at all), not something this task's
//! type additions move. This measurement instead asks the question the task
//! actually poses: for every type Ply's own signature reader
//! (`ply_core::harness::rust_type_from_source`) sees on this crate's public
//! surface, does its codegen accept it? -- the fallback the task's own brief
//! names for exactly this situation.
//!
//! **"Public surface" widened to include `pub(crate)` (sampling/proving
//! split task, 2026-08-27).** This crate's own floating-point refill
//! arithmetic (`Quota::tokens_per_nanosecond -> f64`) is `pub(crate)`, not
//! `pub` -- deliberately: it is the one property
//! `docs/greenfield-ratelimiter-design.md` names as the thing its author
//! trusted least, and hiding it behind a crate-internal accessor is
//! ordinary API design, not an attempt to dodge measurement. A `ply.yaml`
//! claim is written from inside the crate it claims, so `pub(crate)` is
//! real surface such a claim could name; a measurement that only ever
//! looked at `pub` items would report `fuzz_only == 0` forever regardless
//! of what the split could do, which is exactly what it did before this
//! widening (`is_pub_or_crate_visible`, below) -- not evidence the feature
//! changed nothing, but blindness to the one function it was aimed at.

use std::collections::BTreeMap;
use std::path::Path;

use ply_core::harness::rust_type_from_source;
use syn::{FnArg, ReturnType, Visibility};

/// One parameter or return type found on the crate's public surface, as its
/// exact source spelling plus which function it came from (for a readable
/// report, not for correctness).
struct TypeUse {
    owner: String,
    ty_src: String,
}

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// `pub` **or** `pub(crate)` -- widened for the sampling/proving split
/// (task, 2026-08-27), and the reason is not cosmetic: this fixture's own
/// design puts its floating-point refill arithmetic behind exactly
/// `pub(crate)` (`Quota::tokens_per_nanosecond` -> `f64`, `RefillRate`'s own
/// impl block) -- the one property `docs/greenfield-ratelimiter-design.md`
/// names as the thing its author trusted least. A measurement that only
/// ever looks at `pub` items is blind to the exact function this whole
/// feature exists to reach, and `fuzz_only` staying zero forever was that
/// blindness showing, not evidence the split changed nothing. `fns:` items
/// in this crate's own `ply.yaml` can be crate-internal too (a claim is
/// written from inside the crate it claims), so `pub(crate)` is a real part
/// of "this crate's own surface Ply would want to check" -- not a truly
/// private helper nobody would ever anchor a claim to. Left narrow on
/// purpose: only bare `pub(crate)`, never `pub(super)`/`pub(in ...)`, which
/// this fixture does not use and which would need their own path-relative
/// reasoning this measurement has no reason to take on.
fn is_pub_or_crate_visible(vis: &Visibility) -> bool {
    match vis {
        Visibility::Public(_) => true,
        Visibility::Restricted(r) => r.path.is_ident("crate"),
        Visibility::Inherited => false,
    }
}

/// Collects every parameter type (receiver excluded) and every explicit
/// return type from the crate's public-**or-crate-visible** functions:
/// top-level `pub`/`pub(crate) fn`s, every fn in a `pub trait`'s body (its
/// methods are public whenever the trait is, regardless of the `pub`
/// keyword, which traits never repeat on their own items), and every fn in
/// an `impl` block for a type this enumeration has already seen declared
/// `pub` (an inherent impl's fn kept only if it itself says `pub`/
/// `pub(crate)`; a trait impl's fn always kept, since a trait impl's own
/// visibility is the trait's). See `is_pub_or_crate_visible`'s own doc for
/// why crate-visible items are in scope here at all.
fn collect_type_uses(file: &syn::File, owner_prefix: &str, out: &mut Vec<TypeUse>) {
    use syn::Item;

    let mut pub_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for item in &file.items {
        if let Item::Struct(s) = item
            && is_pub(&s.vis)
        {
            pub_types.insert(s.ident.to_string());
        }
    }

    let push_sig = |owner: String,
                    inputs: &syn::punctuated::Punctuated<FnArg, syn::Token![,]>,
                    output: &ReturnType,
                    out: &mut Vec<TypeUse>| {
        for arg in inputs {
            if let FnArg::Typed(pt) = arg {
                out.push(TypeUse {
                    owner: owner.clone(),
                    ty_src: quote::ToTokens::to_token_stream(&*pt.ty).to_string(),
                });
            }
            // `FnArg::Receiver` (`self`/`&self`/`&mut self`) is deliberately
            // never counted: it is never a `kani::any()`/proptest value Ply
            // builds, in this crate or any other -- it names which
            // instance, not an input type.
        }
        if let ReturnType::Type(_, ty) = output {
            out.push(TypeUse {
                owner,
                ty_src: quote::ToTokens::to_token_stream(&**ty).to_string(),
            });
        }
    };

    for item in &file.items {
        match item {
            Item::Fn(f) if is_pub_or_crate_visible(&f.vis) => {
                push_sig(
                    format!("{owner_prefix}{}", f.sig.ident),
                    &f.sig.inputs,
                    &f.sig.output,
                    out,
                );
            }
            Item::Trait(t) if is_pub(&t.vis) => {
                for ti in &t.items {
                    if let syn::TraitItem::Fn(m) = ti {
                        push_sig(
                            format!("{owner_prefix}{}::{}", t.ident, m.sig.ident),
                            &m.sig.inputs,
                            &m.sig.output,
                            out,
                        );
                    }
                }
            }
            Item::Impl(imp) => {
                // The bare type name (`KeyedRateLimiter`, never
                // `KeyedRateLimiter < K , C , S >`) -- generic parameters on
                // the `impl` block itself are part of `self_ty`'s token
                // stream too, and matching the full stream against
                // `pub_types`'s bare idents silently dropped every generic
                // type's methods the first time this ran.
                let self_ty = match &*imp.self_ty {
                    syn::Type::Path(tp) => tp
                        .path
                        .segments
                        .last()
                        .map(|s| s.ident.to_string())
                        .unwrap_or_default(),
                    other => quote::ToTokens::to_token_stream(other).to_string(),
                };
                // Only for a type this file itself declares `pub` (skips
                // impls for foreign types, which have no bearing on this
                // crate's own public surface); a trait impl (`imp.trait_`
                // is `Some`) is public regardless of the type's own
                // visibility qualifier on each method, since the trait
                // itself carries it.
                let is_trait_impl = imp.trait_.is_some();
                if !pub_types.contains(&self_ty) && !is_trait_impl {
                    continue;
                }
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(m) = ii
                        && (is_trait_impl || is_pub_or_crate_visible(&m.vis))
                    {
                        push_sig(
                            format!("{owner_prefix}{self_ty}::{}", m.sig.ident),
                            &m.sig.inputs,
                            &m.sig.output,
                            out,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// `crates/ply-core/` -> the repository root -> the shared fixture tree.
///
/// This measurement lives beside the classifier it measures rather than in
/// the end-to-end suite, where it started. `e2e` exists to drive the built
/// binary the way a user does, so linking the library from it broke the one
/// rule that suite has -- which Ply itself reported the moment it was
/// pointed at this repository (`A0401`, ARCHITECTURE.md). Nothing about the
/// measurement wanted to be end-to-end: it reads one function of
/// `harness`'s and counts what it returns.
fn ratelimiter_src_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("crates/ply-core lives two levels below the repository root")
        .join("tests/fixtures/ratelimiter/src")
}

#[test]
fn ratelimiter_public_surface_type_mapping_coverage() {
    let src_dir = ratelimiter_src_dir();
    assert!(
        src_dir.is_dir(),
        "expected the ratelimiter fixture at {}",
        src_dir.display()
    );

    let mut uses = Vec::new();
    for entry in std::fs::read_dir(&src_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let module = path.file_stem().unwrap().to_str().unwrap().to_string();
        let src = std::fs::read_to_string(&path).unwrap();
        // `#[cfg(test)]` modules are not the crate's public surface --
        // strip them textually before parsing so their helper fns (`fn
        // nz(n: u32) -> NonZeroU32`, `fn quota(...)`) never inflate the
        // count of what a *caller* of this crate actually sees.
        let src = strip_test_modules(&src);
        let file: syn::File = syn::parse_str(&src)
            .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()));
        collect_type_uses(&file, &format!("{module}::"), &mut uses);
    }

    assert!(
        !uses.is_empty(),
        "expected to find public function signatures in the ratelimiter fixture"
    );

    let mut bounded = 0usize;
    let mut fuzz_only = 0usize;
    let mut unsupported = 0usize;
    let mut by_type: BTreeMap<String, (usize, &'static str)> = BTreeMap::new();

    for u in &uses {
        let ty = rust_type_from_source(&u.ty_src)
            .unwrap_or_else(|| panic!("{} is not a parseable Rust type: `{}`", u.owner, u.ty_src));
        let bucket = if ty.is_bounded_supported() {
            bounded += 1;
            "bounded+fuzz"
        } else if ty.is_fuzz_supported() {
            fuzz_only += 1;
            "fuzz-only"
        } else {
            unsupported += 1;
            "unsupported"
        };
        let entry = by_type
            .entry(u.ty_src.replace(' ', ""))
            .or_insert((0, bucket));
        entry.0 += 1;
    }

    eprintln!(
        "PLY_TYPE_COVERAGE|total={}|bounded_supported={}|fuzz_only={}|unsupported={}",
        uses.len(),
        bounded,
        fuzz_only,
        unsupported
    );
    let mut rows: Vec<(&String, &(usize, &str))> = by_type.iter().collect();
    rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    for (ty_src, (count, bucket)) in rows {
        eprintln!("PLY_TYPE_COVERAGE_ROW|{ty_src}|{count}|{bucket}");
    }
}

/// Textually removes every `#[cfg(test)] mod ... { ... }` block by brace
/// counting -- cheap and exact enough here because the fixture never nests
/// a `{`/`}` inside a string or comment that would confuse the count (true
/// of every file in this crate, read directly).
fn strip_test_modules(src: &str) -> String {
    let marker = "#[cfg(test)]";
    let mut out = String::new();
    let mut rest = src;
    while let Some(pos) = rest.find(marker) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let brace_start = after.find('{').expect("cfg(test) mod has a brace");
        let bytes = after.as_bytes();
        let mut depth = 0i32;
        let mut i = brace_start;
        let end = loop {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break i;
                    }
                }
                _ => {}
            }
            i += 1;
        };
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}
