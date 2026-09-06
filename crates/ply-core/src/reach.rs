//! The code one claim's result actually stood on (The-Ply-Spec.md §5.2a).
//!
//! A recorded result is reused when a hash of what it depended on still
//! matches. Until 2026-08-25 that hash covered the checked function's own
//! tokens and the promises declared for the callees a proof was allowed to
//! replace -- and nothing else. A plain local helper the check *ran*, or a
//! contracted callee the proof *descended into*, was invisible to it, so
//! breaking the helper and re-running produced a confident carried-forward
//! pass over code a cold run proves is in violation. This module is what
//! closes that: it answers, for one claim, **which first-party bodies can
//! this check reach**, and hands them to the fingerprint.
//!
//! It answers that in one of two ways, and says which:
//!
//! - **reached** -- Ply followed every path out of the claimed function and
//!   can name the whole set. Editing an unrelated function in the same
//!   crate then costs nothing, which is the granularity the record exists
//!   for.
//! - **whole-crate** -- Ply could *not* bound the walk, so it hashes every
//!   line of first-party source instead. Coarser: any edit anywhere in the
//!   crate re-earns every claim in it. Never wrong.
//!
//! **Why the second mode has to exist.** A syntactic walk can follow a call
//! written as a call. It cannot follow `x.helper()` (which body that names
//! depends on the receiver's type), an operator (`a + b` runs a first-party
//! `impl Add` if one exists), a macro (whose expansion is not in the token
//! stream the walk sees), or a trait method reached through a blanket impl
//! under a different name (`x.into()` runs somebody's `from`). Resolving
//! those needs a type checker, which Ply is not. So the walk is trusted
//! only under conditions that make all of them impossible: **every item in
//! first-party source is a function, a module, an import, a type alias, or
//! a plain data type**, no reached body invokes a macro, and no reached
//! function carries an attribute Ply does not recognise. No `impl` block
//! anywhere means no method and no operator can land in first-party code;
//! no macro in a reached body means no call is hidden from the walk; no
//! `const`/`static` means no initializer runs code the walk never sees; no
//! unfamiliar attribute means no body was rewritten into something else
//! before it ran. When any of that fails, the walk is abandoned rather than
//! trimmed, and the whole crate is hashed.
//!
//! Two things the walk follows that a plain call graph would not, because
//! both run code: a function named as a *value* (`map(helper)` never writes
//! `helper(..)`), and a function named inside the claim's own **contract**
//! (`#[ply::ensures(|result| *result == expected(x))]` runs `expected` on
//! every generated case, from an attribute no walk of the body would see).
//!
//! The condition is an **allowlist**, deliberately: an item kind nobody
//! thought of falls into "widen and hash everything", which costs engine
//! time. A denylist would have put it in "reuse anyway", which costs the
//! user a green verdict over code nobody checked. That direction is the
//! whole point.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::visit::Visit;

use crate::callgraph::{CallSite, CalleeStatus, Resolution, Resolver};

/// How far a claim's `bounded` proof or generated test can reach into
/// first-party code, as the fingerprint records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeScope {
    /// `reached` or `whole-crate`. Hashed, so a claim that stops being
    /// bounded-by-a-walk never matches a record written while it was.
    pub scope: &'static str,
    /// `(label, token text)` for every body in scope, ordered by label.
    /// Labels are workspace-relative, never absolute paths: two checkouts
    /// of the same code must hash the same.
    pub units: Vec<(String, String)>,
    /// One plain sentence naming what stopped the walk, when it stopped.
    pub widened_because: Option<String>,
}

/// Every first-party source file in reach of this crate, parsed once per
/// run: the crate's own `src/` tree and that of each path dependency,
/// transitively.
pub struct FirstParty {
    /// `(label, token text)` per file, ordered by label.
    units: Vec<(String, String)>,
    /// `Some(reason)` when first-party source contains something a
    /// syntactic call walk cannot bound -- see the module comment.
    gate: Option<String>,
    /// `(label, token text)` for every type declaration in first-party
    /// source: the aliases, structs and enums the gate lets through.
    ///
    /// A walk of *bodies* can never reach these, and they were therefore
    /// not hashed at all -- while changing one changes what every body
    /// around it means. `type Input = u8;` widened to `u16` left every
    /// function byte-identical and made `|result| *result <= 255` false,
    /// and the record matched, so Ply reported a green it had earned over
    /// a strictly smaller input space (reproduced end to end, 2026-09-05:
    /// cached `fuzzed(256) [reused]`, fresh `violation` at `x = 256`).
    ///
    /// Hashed as one set rather than per-reached-type on purpose: working
    /// out which declarations a body depends on needs the type checker Ply
    /// is not, so an unused struct changing re-earns the crate's claims.
    /// That is this module's stated trade -- coarser, never wrong.
    type_decls: Vec<(String, String)>,
}

impl FirstParty {
    pub fn gate(&self) -> Option<&str> {
        self.gate.as_deref()
    }
}

/// The item kinds a call walk can bound. Anything else -- an `impl`, a
/// `trait`, a `const`, a `static`, a `macro_rules!`, an `extern` block --
/// can put first-party code behind a method call, an operator, or an
/// initializer, none of which a syntactic walk follows.
/// A plain type path spelled as source (`super::Quota`), not as a token
/// stream (`super :: Quota`). Any other type shape falls back to the token
/// rendering, which is at least accurate.
fn type_path_source(ty: &syn::Type) -> String {
    if let syn::Type::Path(tp) = ty
        && tp.qself.is_none()
    {
        return tp
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
    }
    ty.to_token_stream().to_string()
}

fn item_is_walkable(item: &syn::Item, label: &str) -> Result<(), String> {
    let named = |what: &str, name: String| {
        Err(format!(
            "{label} declares {what} `{name}`, and Ply cannot tell by reading the source \
             which of its bodies a method call or an operator would run"
        ))
    };
    match item {
        syn::Item::Fn(_) | syn::Item::Use(_) | syn::Item::Type(_) => Ok(()),
        syn::Item::Struct(s) => derives_are_plain(&s.attrs, label, &s.ident.to_string()),
        syn::Item::Enum(e) => derives_are_plain(&e.attrs, label, &e.ident.to_string()),
        syn::Item::Mod(m) => match &m.content {
            None => Ok(()),
            Some((_, items)) => {
                for inner in items {
                    item_is_walkable(inner, label)?;
                }
                Ok(())
            }
        },
        // Spelled the way the user wrote it. `to_token_stream()` renders a
        // qualified path as `super :: Quota` -- accurate, and not a phrase
        // any sentence held to the newbie bar should quote at someone.
        syn::Item::Impl(i) => named("an `impl` block for", type_path_source(&i.self_ty)),
        syn::Item::Trait(t) => named("a trait", t.ident.to_string()),
        syn::Item::Const(c) => named("a constant", c.ident.to_string()),
        syn::Item::Static(s) => named("a static", s.ident.to_string()),
        syn::Item::Macro(m) => Err(format!(
            "{label} declares or invokes a macro at the top level, and what a macro expands \
             to is not in the source Ply reads: {}",
            m.mac.path.to_token_stream()
        )),
        other => Err(format!(
            "{label} declares `{}`, an item kind Ply does not know how to follow calls out of",
            first_tokens(other)
        )),
    }
}

/// The attributes a **walked** function may carry without putting code
/// beyond the walk's reach. Anything else may be an attribute macro, whose
/// expansion can replace the body with anything at all -- so a function
/// carrying one is refused and the whole crate is hashed instead.
/// `cfg_attr` is deliberately absent: it can apply any attribute at all,
/// conditionally.
///
/// Checked per reached function rather than over the whole crate, because
/// it is a fact about the body the walk is about to read. A macro on a
/// function nothing reaches changes nothing about this claim -- and a
/// crate-wide check would fire on every `#[test]` in a dependency.
const INERT_FN_ATTRS: [&str; 9] = [
    "doc", "inline", "cfg", "allow", "deny", "warn", "expect", "must_use", "cold",
];

/// `#[ply::requires]` / `#[ply::ensures]`: Ply's own, and the two whose
/// argument is an expression that **runs** -- so the functions it names are
/// functions the check executes, and the walk has to follow them.
fn is_contract_attr(attr: &syn::Attribute) -> bool {
    let segs: Vec<String> = attr
        .path()
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    matches!(
        segs.last().map(String::as_str),
        Some("requires") | Some("ensures")
    ) && (segs.len() == 1 || segs.first().map(String::as_str) == Some("ply"))
}

fn attributes_are_inert(attrs: &[syn::Attribute], name: &str) -> Result<(), String> {
    for attr in attrs {
        if is_contract_attr(attr) {
            continue;
        }
        let path = attr
            .path()
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        if !INERT_FN_ATTRS.contains(&path.as_str()) {
            return Err(format!(
                "`{name}` carries `#[{path}]`, which may be an attribute macro, and Ply cannot \
                 read what a macro expands the body into"
            ));
        }
    }
    Ok(())
}

/// The derives that can only ever generate `std` behaviour over the fields.
/// A derive from anywhere else is somebody's proc macro, and its expansion
/// can call anything.
const PLAIN_DERIVES: [&str; 9] = [
    "Debug",
    "Clone",
    "Copy",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Default",
];

fn derives_are_plain(attrs: &[syn::Attribute], label: &str, name: &str) -> Result<(), String> {
    for attr in attrs {
        if !attr.path().is_ident("derive") {
            continue;
        }
        let mut offending: Option<String> = None;
        let _ = attr.parse_nested_meta(|meta| {
            let ident = meta
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if !PLAIN_DERIVES.contains(&ident.as_str()) {
                offending.get_or_insert(ident);
            }
            Ok(())
        });
        if let Some(what) = offending {
            return Err(format!(
                "`{name}` in {label} derives `{what}`, whose expansion Ply cannot read and which \
                 may put first-party code behind a method call"
            ));
        }
    }
    Ok(())
}

fn first_tokens(item: &syn::Item) -> String {
    item.to_token_stream()
        .to_string()
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A module Ply itself writes into the crate under check, and which must
/// never count as the user's code: hashing it would make every run after a
/// proof invalidate the results of the one before it.
fn is_ply_generated(name: &str) -> bool {
    name == "ply_generated" || name == "ply_generated_cex"
}

/// Reads every first-party source file once and decides whether a call walk
/// can be trusted over this crate at all.
pub fn scan_first_party(crate_dir: &Path) -> FirstParty {
    let mut units: Vec<(String, String)> = Vec::new();
    let mut type_decls: Vec<(String, String)> = Vec::new();
    let mut gate: Option<String> = None;
    for (label, path) in first_party_files(crate_dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            gate.get_or_insert(format!("Ply could not read {label}"));
            continue;
        };
        let Ok(file) = syn::parse_file(&text) else {
            // Unreadable as Rust, so hash the bytes and stop trusting the
            // walk: "I could not look" is not "there is nothing there".
            gate.get_or_insert(format!("Ply could not parse {label} as Rust"));
            units.push((label, text));
            continue;
        };
        let mut tokens = proc_macro2::TokenStream::new();
        for item in &file.items {
            if let syn::Item::Mod(m) = item
                && is_ply_generated(&m.ident.to_string())
            {
                continue;
            }
            if let Err(reason) = item_is_walkable(item, &label) {
                gate.get_or_insert(reason);
            }
            collect_type_decls(item, &label, &mut type_decls);
            item.to_tokens(&mut tokens);
        }
        units.push((label, tokens.to_string()));
    }
    units.sort();
    type_decls.sort();
    FirstParty {
        units,
        gate,
        type_decls,
    }
}

/// Every type declaration in one item, including inside modules, as
/// `(label, token text)`.
///
/// Only the kinds `item_is_walkable` lets past its gate reach this: an
/// alias, a struct, an enum. Anything else has already widened the scope to
/// the whole crate, where the file's own tokens are hashed regardless.
fn collect_type_decls(item: &syn::Item, label: &str, out: &mut Vec<(String, String)>) {
    match item {
        syn::Item::Type(t) => out.push((
            format!("{label}::{}", t.ident),
            t.to_token_stream().to_string(),
        )),
        syn::Item::Struct(t) => out.push((
            format!("{label}::{}", t.ident),
            t.to_token_stream().to_string(),
        )),
        syn::Item::Enum(t) => out.push((
            format!("{label}::{}", t.ident),
            t.to_token_stream().to_string(),
        )),
        syn::Item::Mod(m) => {
            if let Some((_, items)) = &m.content {
                for inner in items {
                    collect_type_decls(inner, &format!("{label}::{}", m.ident), out);
                }
            }
        }
        _ => {}
    }
}

/// The code one claim's checks can reach, as the fingerprint records it.
///
/// `stubbed` names the callees this claim's proof replaces with a declared
/// promise (§5.5's second branch). Their bodies are not what the result
/// stood on -- the promise is, and the promise is hashed separately -- so
/// the walk stops there. It must be **empty** whenever any check in the
/// claim runs the real code (`fuzz`, `test`, `mutate`), because those tiers
/// call the callee for real however many promises are declared for it.
pub fn code_scope(
    resolver: &mut Resolver,
    first_party: &FirstParty,
    root_fn_path: &str,
    expressions: &[String],
    stubbed: &BTreeSet<String>,
) -> CodeScope {
    if let Some(reason) = &first_party.gate {
        return widened(first_party, reason.clone());
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    // Seeded with every first-party type declaration, because no walk of
    // bodies can reach one and changing one changes what the bodies mean.
    // See `FirstParty::type_decls`.
    let mut units: Vec<(String, String)> = first_party.type_decls.clone();
    queue.push_back(root_fn_path.to_string());
    // A worked example is code: a `test` check compiles it into an assertion
    // and runs it. So is a contract written in `ply.yaml` -- it is merged in
    // after the function item is read, so it never appears among the
    // attributes the walk above reads, and a helper it names was hashed
    // nowhere at all. Both arrive here as expression text.
    //
    // So anything either names is part of what the result stood on,
    // exactly as a callee of the function itself is. Walking out of the
    // claimed function alone missed a helper the example called and the
    // function never mentioned -- `rate(x) == expected()` with `expected`
    // defined next door. Editing that helper changed what the assertion
    // demanded and left the fingerprint identical, so the run carried forward
    // a pass over a check that had just changed meaning.
    //
    // Reported by external review, 2026-09-06.
    for example in expressions {
        let Ok(expr) = syn::parse_str::<syn::Expr>(example) else {
            // The harness refuses this text too, but that happens later, and
            // a scope that quietly skips what it cannot read is the silence
            // this module exists to end.
            return widened(
                first_party,
                format!("Ply could not read the worked example `{example}` to walk out of it"),
            );
        };
        let mut collector = MentionCollector {
            paths: Vec::new(),
            macro_invocation: None,
        };
        collector.visit_expr(&expr);
        if let Some(mac) = collector.macro_invocation {
            return widened(
                first_party,
                format!(
                    "the worked example `{example}` invokes the macro `{mac}!`, whose expansion \
                     is not in the tokens Ply's call walk reads"
                ),
            );
        }
        for path in collector.paths {
            queue.push_back(path);
        }
    }
    while let Some(spelling) = queue.pop_front() {
        let is_root = seen.is_empty();
        let found = match resolver.lookup_fn(&spelling) {
            Resolution::Found(f) => f,
            Resolution::Opaque(reason) => return widened(first_party, reason),
            // A path that names nothing is ordinary further down the walk:
            // most mentioned paths are local variables. At the **root** it
            // means Ply is about to hash an empty set of reachable bodies
            // for a function it cannot even find, which reads as "this
            // check runs nothing" -- the exact silence this module exists
            // to end. Widen instead.
            Resolution::NotFound if is_root => {
                return widened(
                    first_party,
                    format!("Ply could not resolve `{root_fn_path}` to walk out of it"),
                );
            }
            Resolution::NotFound => continue,
            // A method Ply resolves and refuses (a receiver, a generic
            // `impl`, a trait method) or cannot choose between (an
            // ambiguous `impl` match): out of this task's scope to descend
            // into either way, so this widens the same way an unreadable
            // root would, and is skipped the same way an unresolved callee
            // further down the walk already is.
            Resolution::Refused(reason) | Resolution::Ambiguous(reason) if is_root => {
                return widened(first_party, reason);
            }
            Resolution::Refused(_) | Resolution::Ambiguous(_) => continue,
        };
        if !seen.insert(found.canonical.clone()) {
            continue;
        }
        // The claimed function's own tokens are a hashed input in their own
        // right, and hashing them twice would make one edit report as two
        // inputs moving ("the function's own source *and* the code it
        // runs"). The explanation a user reads has to name what actually
        // changed.
        if !is_root {
            units.push((
                found.canonical.clone(),
                found.item.to_token_stream().to_string(),
            ));
        }
        if let Err(reason) = attributes_are_inert(&found.item.attrs, &found.canonical) {
            return widened(first_party, reason);
        }
        let mentions = mentioned_paths(&found.item);
        if let Some(mac) = mentions.macro_invocation {
            return widened(
                first_party,
                format!(
                    "`{}` invokes the macro `{mac}!`, whose expansion is not in the tokens Ply's \
                     call walk reads",
                    found.canonical
                ),
            );
        }
        // A name written inside a module is resolved from that module
        // first, then from the crate root -- the way Rust reads it. Without
        // this, a bare `helper(x)` inside `mod maths` resolved to nothing,
        // and "nothing" fell into the `Unresolved` arm below, whose comment
        // says "out of the workspace: `std`, or a registry crate". It was
        // neither: it was the function on the next line. Editing it left
        // the caller's fingerprint unchanged and Ply carried a green
        // forward over code a cold run reports as a violation
        // (reproduced end to end, 2026-09-05).
        let module_prefix = found
            .canonical
            .rsplit_once("::")
            .map(|(module, _)| module.to_string());
        for path in mentions.paths {
            // Module-qualified first, bare second. The first spelling that
            // resolves to anything at all is the one Rust would have run;
            // only if neither resolves is this genuinely outside.
            let qualified = module_prefix
                .as_ref()
                .map(|m| format!("{m}::{path}"))
                .filter(|q| {
                    !matches!(
                        resolver
                            .classify(&CallSite {
                                path: q.clone(),
                                line: 0,
                                col: 0,
                            })
                            .status,
                        CalleeStatus::Unresolved
                    )
                });
            let path = qualified.unwrap_or(path);
            let site = CallSite {
                path: path.clone(),
                line: 0,
                col: 0,
            };
            match resolver.classify(&site).status {
                // First-party source Ply was pointed at and could not read.
                CalleeStatus::Opaque(reason) => return widened(first_party, reason),
                // Out of the workspace: `std`, or a registry crate. Not
                // hashable as source, and covered instead by the compiler
                // identity and the resolved dependency versions (§5.2a).
                CalleeStatus::Unresolved => {}
                // Replaced by a promise for this claim: the proof never saw
                // the body, so the body is not what the result stood on.
                CalleeStatus::Assumed {
                    ref canonical_path, ..
                } if stubbed.contains(canonical_path) => {}
                _ => queue.push_back(path),
            }
        }
    }
    units.sort();
    CodeScope {
        scope: "reached",
        units,
        widened_because: None,
    }
}

fn widened(first_party: &FirstParty, reason: String) -> CodeScope {
    CodeScope {
        scope: "whole-crate",
        units: first_party.units.clone(),
        widened_because: Some(reason),
    }
}

/// Every path a body *mentions*, and whether it invokes a macro.
///
/// Paths, not just call sites: `helper(x)` and `map(helper)` both put
/// `helper`'s body in the run, and only the first is a call expression.
/// Collecting both over-approximates (a local variable that shares a
/// function's name resolves to that function and gets hashed), which costs
/// a little invalidation and never a false reuse.
struct Mentions {
    paths: Vec<String>,
    macro_invocation: Option<String>,
}

struct MentionCollector {
    paths: Vec<String>,
    macro_invocation: Option<String>,
}

impl<'ast> Visit<'ast> for MentionCollector {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        let path = node
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        if !path.is_empty() && !self.paths.contains(&path) {
            self.paths.push(path);
        }
        syn::visit::visit_expr_path(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if self.macro_invocation.is_none() {
            self.macro_invocation = Some(node.path.to_token_stream().to_string());
        }
        syn::visit::visit_macro(self, node);
    }
}

fn mentioned_paths(f: &syn::ItemFn) -> Mentions {
    let mut c = MentionCollector {
        paths: Vec::new(),
        macro_invocation: None,
    };
    c.visit_block(&f.block);
    // A contract is code too. `#[ply::ensures(|result| *result ==
    // expected(x))]` runs `expected` on every generated case, so a helper
    // named only in a contract is a helper the check executes -- and it
    // lives in an attribute, which no walk of the body would ever see.
    for attr in &f.attrs {
        if !is_contract_attr(attr) {
            continue;
        }
        match attr.parse_args::<syn::Expr>() {
            Ok(expr) => c.visit_expr(&expr),
            // Unreadable as an expression: refuse rather than skip, by
            // reporting it as a macro, which is what widens the scope.
            Err(_) => {
                if c.macro_invocation.is_none() {
                    c.macro_invocation = Some("an unreadable contract expression".into());
                }
            }
        }
    }
    Mentions {
        paths: c.paths,
        macro_invocation: c.macro_invocation,
    }
}

/// `(label, path)` for every `.rs` file under this crate's `src/` and under
/// each path dependency's, transitively. Labels are relative to the
/// workspace, so the same code hashes the same in a different checkout.
fn first_party_files(crate_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut seen_crates: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: VecDeque<(String, PathBuf)> = VecDeque::new();
    queue.push_back((String::new(), crate_dir.to_path_buf()));
    while let Some((prefix, dir)) = queue.pop_front() {
        let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !seen_crates.insert(canonical) {
            continue;
        }
        collect_rs(&dir.join("src"), &format!("{prefix}src"), &mut out);
        // A build script is code the build runs, and what it emits reaches
        // the checked crate: `cargo:rustc-env=ANSWER=7` makes `env!("ANSWER")`
        // compile to 7, so editing the script changes behaviour with every
        // line under `src/` untouched. Collecting only `src/` left that edit
        // invisible to the fingerprint, and a cached pass survived it.
        //
        // KNOWN GAP, stated rather than left to be found: this hashes the
        // script, not what the script reads. A build script that opens a
        // data file and emits what it finds there still changes behaviour
        // without changing anything hashed here.
        let build_rs = dir.join("build.rs");
        if build_rs.is_file() {
            out.push((format!("{prefix}build.rs"), build_rs));
        }
        if let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) {
            for (name, rel) in path_dependencies(&manifest) {
                queue.push_back((format!("{name}/"), dir.join(rel)));
            }
        }
    }
    out.sort();
    out
}

fn collect_rs(dir: &Path, label: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
    names.sort();
    for name in names {
        let name = name.to_string_lossy().into_owned();
        let path = dir.join(&name);
        let child_label = format!("{label}/{name}");
        if path.is_dir() {
            collect_rs(&path, &child_label, out);
        } else if let Some(stem) = name.strip_suffix(".rs")
            && !is_ply_generated(stem)
        {
            out.push((child_label, path));
        }
    }
}

/// The three tables Cargo builds a dependency graph from. `build-` counts
/// because a build script's own dependencies compile and run during the
/// build, and can write the source the crate then compiles.
const DEPENDENCY_KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// Whether a `Cargo.toml` table header declares dependencies -- and, when it
/// is the one-table-per-dependency form, which dependency it names.
///
/// Cargo spells the same table several ways, and Ply recognised two of them.
/// `[target.'cfg(unix)'.dependencies]` and `[build-dependencies]` were read
/// as ordinary tables of nothing, so a path dependency declared under either
/// was never walked and none of its source was hashed. Editing a helper
/// there and re-running carried a pass forward over code that had changed.
///
/// The platform predicate can contain dots, quotes and parentheses
/// (`target."cfg(any(unix, windows))".dependencies`), so the prefix is
/// dropped by finding the kind rather than by splitting on `.`.
pub(crate) fn dependency_table(header: &str) -> Option<Option<String>> {
    let inner = header.strip_prefix('[')?.strip_suffix(']')?;
    let rest = match inner.strip_prefix("target.") {
        Some(after) => {
            let at = DEPENDENCY_KINDS
                .iter()
                .filter_map(|kind| after.rfind(&format!(".{kind}")))
                .max()?;
            &after[at + 1..]
        }
        None => inner,
    };
    for kind in DEPENDENCY_KINDS {
        if rest == kind {
            return Some(None);
        }
        if let Some(name) = rest.strip_prefix(&format!("{kind}.")) {
            return Some(Some(name.to_string()));
        }
    }
    None
}

/// `(dependency key, relative path)` for every `path = "..."` dependency in
/// a manifest. The same deliberately narrow line scan the rest of Ply uses
/// on `Cargo.toml`: this text is read for two keys, never interpreted.
pub fn path_dependencies(manifest: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_deps = false;
    let mut pending: Option<String> = None;
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            match dependency_table(t) {
                Some(named) => {
                    in_deps = true;
                    pending = named;
                }
                None => {
                    in_deps = false;
                    pending = None;
                }
            }
            continue;
        }
        if !in_deps || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(key) = &pending {
            if let Some(p) = quoted_value(t, "path") {
                out.push((key.clone(), p));
            }
            continue;
        }
        let Some((key, rest)) = t.split_once('=') else {
            continue;
        };
        if let Some(p) = quoted_value(rest, "path") {
            out.push((key.trim().to_string(), p));
        }
    }
    out
}

fn quoted_value(text: &str, key: &str) -> Option<String> {
    let at = text.find(key)?;
    let rest = &text[at + key.len()..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The identity of everything the check runs that is **not** first-party
/// source: the registry crates this crate resolves to, at the versions the
/// lockfile pins.
///
/// A `bounded` proof descends into `std` and into registry code (§5.5 states
/// that as the rule's gap), and every `fuzz`/`test` run executes it. The
/// compiler identity already covers `std`; this covers the rest. Reachable
/// from the target package only, so the harness crate Ply generates -- which
/// depends on the target, never the reverse -- cannot move it.
pub fn dependency_identity(crate_dir: &Path) -> String {
    let package = std::fs::read_to_string(crate_dir.join("Cargo.toml"))
        .ok()
        .and_then(|t| crate::harness_crate::read_crate_names(&t).ok())
        .map(|n| n.package_name);
    match (lockfile(crate_dir), package) {
        (Some(text), Some(package)) => {
            let pinned = registry_packages_reachable_from(&text, &package);
            if pinned.is_empty() {
                NO_EXTERNAL_CODE.to_string()
            } else {
                pinned.join("\n")
            }
        }
        _ => {
            if has_registry_dependency(crate_dir) {
                // Stated rather than guessed: without a lockfile Ply cannot
                // know which versions an earlier run compiled against, so a
                // result recorded with one never matches a run without one.
                "(no Cargo.lock: the resolved dependency versions are not known)".to_string()
            } else {
                NO_EXTERNAL_CODE.to_string()
            }
        }
    }
}

const NO_EXTERNAL_CODE: &str = "(nothing outside this workspace)";

fn lockfile(crate_dir: &Path) -> Option<String> {
    let mut dir = Some(crate_dir);
    while let Some(d) = dir {
        let candidate = d.join("Cargo.lock");
        if candidate.is_file() {
            return std::fs::read_to_string(candidate).ok();
        }
        dir = d.parent();
    }
    None
}

/// Whether any manifest in this crate's path-dependency closure names a
/// dependency that is not itself a path dependency.
fn has_registry_dependency(crate_dir: &Path) -> bool {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(crate_dir.to_path_buf());
    while let Some(dir) = queue.pop_front() {
        let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !seen.insert(canonical) {
            continue;
        }
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let paths = path_dependencies(&manifest);
        if declared_dependency_count(&manifest) > paths.len() {
            return true;
        }
        for (_, rel) in paths {
            queue.push_back(dir.join(rel));
        }
    }
    false
}

fn declared_dependency_count(manifest: &str) -> usize {
    let mut count = 0;
    let mut in_deps = false;
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            match dependency_table(t) {
                // `[dependencies.serde]` is one dependency spelled over
                // several lines, not several dependencies.
                Some(Some(_)) => {
                    count += 1;
                    in_deps = false;
                }
                Some(None) => in_deps = true,
                None => in_deps = false,
            }
            continue;
        }
        if in_deps && !t.is_empty() && !t.starts_with('#') && t.contains('=') {
            count += 1;
        }
    }
    count
}

/// `name version` for every package with a `source` (i.e. not in this
/// workspace) reachable from `root` in a `Cargo.lock`.
/// The one registry whose published versions are immutable by policy, so
/// `name version` already names exact bytes.
const CRATES_IO_REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

fn registry_packages_reachable_from(lock: &str, root: &str) -> Vec<String> {
    struct Pkg {
        name: String,
        version: String,
        /// The lockfile's own `source =` line, kept whole rather than
        /// reduced to "is this external".
        ///
        /// It carries the revision for a git dependency
        /// (`git+https://...?branch=main#<sha>`), and collapsing it to a
        /// boolean meant two builds of the same package version from
        /// different revisions produced one identity -- so updating a git
        /// dependency without bumping its version left every fingerprint
        /// unchanged and every recorded green reusable over code that had
        /// moved underneath it.
        source: Option<String>,
        deps: Vec<String>,
    }
    let mut packages: std::collections::BTreeMap<String, Pkg> = std::collections::BTreeMap::new();
    let mut by_name: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut name = String::new();
    let mut version = String::new();
    let mut source: Option<String> = None;
    let mut deps: Vec<String> = Vec::new();
    let mut in_deps_list = false;
    let mut started = false;
    let flush = |name: &mut String,
                 version: &mut String,
                 source: &mut Option<String>,
                 deps: &mut Vec<String>,
                 packages: &mut std::collections::BTreeMap<String, Pkg>,
                 by_name: &mut std::collections::BTreeMap<String, Vec<String>>| {
        if !name.is_empty() {
            // Keyed by name *and* version. A lockfile may legitimately hold
            // two versions of one crate; keyed by name alone the second
            // block overwrote the first, so the identity could name a
            // version this crate never built with and would move when an
            // unrelated crate bumped its own copy.
            let n = std::mem::take(name);
            let v = std::mem::take(version);
            by_name
                .entry(n.clone())
                .or_default()
                .push(format!("{n} {v}"));
            packages.insert(
                format!("{n} {v}"),
                Pkg {
                    name: n,
                    version: v,
                    source: source.take(),
                    deps: std::mem::take(deps),
                },
            );
        }
    };
    for line in lock.lines() {
        let t = line.trim();
        if t == "[[package]]" {
            flush(
                &mut name,
                &mut version,
                &mut source,
                &mut deps,
                &mut packages,
                &mut by_name,
            );
            started = true;
            in_deps_list = false;
            continue;
        }
        if !started {
            continue;
        }
        if in_deps_list {
            if t.starts_with(']') {
                in_deps_list = false;
                continue;
            }
            // The whole entry, not just its first word. Cargo writes
            // `"dep 0.1.0"` exactly when the bare name would be ambiguous,
            // so the version here is the only thing saying which of two
            // copies this crate compiled against.
            let entry = t.trim_matches(|c| c == '"' || c == ',').trim();
            if !entry.is_empty() {
                deps.push(entry.to_string());
            }
            continue;
        }
        if let Some(v) = t.strip_prefix("name = ") {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = t.strip_prefix("version = ") {
            version = v.trim_matches('"').to_string();
        } else if let Some(v) = t.strip_prefix("source = ") {
            source = Some(v.trim_matches('"').to_string());
        } else if t.starts_with("dependencies = [") {
            in_deps_list = !t.ends_with(']');
        }
    }
    flush(
        &mut name,
        &mut version,
        &mut source,
        &mut deps,
        &mut packages,
        &mut by_name,
    );

    // A `dependencies` entry is either `"name"` or `"name version"`. The
    // second form is already a key; the first is one only when the name is
    // unambiguous. When it is not -- which a well-formed lockfile does not
    // produce, but a hand-edited one might -- every candidate is walked, so
    // the identity is coarser than necessary rather than silently wrong.
    let resolve = |entry: &str| -> Vec<String> {
        if packages.contains_key(entry) {
            return vec![entry.to_string()];
        }
        by_name.get(entry).cloned().unwrap_or_default()
    };

    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.extend(resolve(root));
    while let Some(next) = queue.pop_front() {
        if !seen.insert(next.clone()) {
            continue;
        }
        let Some(pkg) = packages.get(&next) else {
            continue;
        };
        if let Some(src) = &pkg.source {
            // Name and version alone, for a crates.io package: a published
            // version there is immutable by that registry's own policy, so
            // the two together already name one exact set of bytes and the
            // source line adds a constant.
            //
            // Everything else carries its source. A git dependency's version
            // says nothing about which commit was built -- the revision is
            // in this line and nowhere else -- and an alternative registry
            // makes no immutability promise this code can rely on.
            //
            // Narrow on purpose. Appending the source unconditionally would
            // have changed the identity of every dependency in the world and
            // invalidated every recorded result, for no gain on the one case
            // that needs nothing: the same reseed-everything mistake a
            // contract-text re-render made earlier today.
            if src == CRATES_IO_REGISTRY {
                out.insert(format!("{} {}", pkg.name, pkg.version));
            } else {
                out.insert(format!("{} {} {src}", pkg.name, pkg.version));
            }
        }
        for d in &pkg.deps {
            queue.extend(resolve(d));
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crate on disk: `src/lib.rs` and whatever else is asked for.
    fn crate_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"c\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        for (rel, text) in files {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn scope_of(dir: &Path, root: &str, stubbed: &[&str]) -> CodeScope {
        scope_of_with_examples(dir, root, &[], stubbed)
    }

    fn scope_of_with_examples(
        dir: &Path,
        root: &str,
        examples: &[&str],
        stubbed: &[&str],
    ) -> CodeScope {
        let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        let mut resolver = Resolver::new(&lib, dir, Default::default()).unwrap();
        let first_party = scan_first_party(dir);
        let stubbed: BTreeSet<String> = stubbed.iter().map(|s| s.to_string()).collect();
        let examples: Vec<String> = examples.iter().map(|s| s.to_string()).collect();
        code_scope(&mut resolver, &first_party, root, &examples, &stubbed)
    }

    fn labels(scope: &CodeScope) -> Vec<String> {
        scope.units.iter().map(|(l, _)| l.clone()).collect()
    }

    /// Cargo spells a dependency table four ways, and Ply recognised two of
    /// them. A path dependency under `[target.'cfg(unix)'.dependencies]` or
    /// `[build-dependencies]` is code the build compiles exactly as one under
    /// `[dependencies]` is, but the walk never entered its `src/`, so nothing
    /// in it was hashed. Editing a helper there and re-running carried
    /// forward a pass over source that had changed.
    ///
    /// Reported by external review, 2026-09-06 (which named the
    /// platform-specific table; the build-script one is the same hole).
    #[test]
    fn a_path_dependency_in_any_dependency_table_is_hashed() {
        for table in [
            "[dependencies]",
            "[dev-dependencies]",
            "[build-dependencies]",
            "[target.'cfg(unix)'.dependencies]",
            "[target.\"cfg(windows)\".dev-dependencies]",
            "[target.x86_64-unknown-linux-gnu.build-dependencies]",
            "[target.'cfg(unix)'.dependencies.helper]",
        ] {
            let entry = if table.ends_with(".helper]") {
                "path = \"helper\"\n".to_string()
            } else {
                "helper = { path = \"helper\" }\n".to_string()
            };
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(
                dir.path().join("Cargo.toml"),
                format!("[package]\nname = \"c\"\nversion = \"0.0.0\"\n\n{table}\n{entry}"),
            )
            .unwrap();
            std::fs::create_dir_all(dir.path().join("src")).unwrap();
            std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\n").unwrap();
            std::fs::create_dir_all(dir.path().join("helper/src")).unwrap();
            std::fs::write(
                dir.path().join("helper/Cargo.toml"),
                "[package]\nname = \"helper\"\nversion = \"0.0.0\"\n",
            )
            .unwrap();
            std::fs::write(dir.path().join("helper/src/lib.rs"), "pub fn b() {}\n").unwrap();

            let found = first_party_files(dir.path());
            assert!(
                found.iter().any(|(label, _)| label.contains("helper/")),
                "{table} declares a path dependency whose source the build compiles, \
                 and it was hashed nowhere: {:?}",
                found.iter().map(|(l, _)| l).collect::<Vec<_>>()
            );
        }
    }

    /// A worked example is code that runs, and it can call a helper the
    /// claimed function never mentions. The walk started at the claimed
    /// function only, so `fn expected() -> u32 { 6 }` -- named by the example
    /// and by nothing else -- was hashed nowhere. Editing it to return a
    /// different answer left the fingerprint identical, and the run carried
    /// forward a pass over an assertion that had just changed meaning.
    ///
    /// Reported by external review, 2026-09-06.
    #[test]
    fn a_helper_only_a_worked_example_calls_is_in_scope() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub fn expected() -> u32 { 6 }\npub fn double(x: u32) -> u32 { x * 2 }\n",
        )]);
        let scope = scope_of_with_examples(dir.path(), "double", &["double(3) == expected()"], &[]);
        assert_eq!(scope.scope, "reached", "{:?}", scope.widened_because);
        assert!(
            labels(&scope).contains(&"expected".to_string()),
            "the example asserts against whatever this returns: {:?}",
            labels(&scope)
        );
    }

    /// And an example naming nothing but literals and the claimed function
    /// must not widen the scope or add units -- the common case stays exact.
    #[test]
    fn an_example_of_plain_literals_leaves_the_scope_alone() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub fn scale(x: u32) -> u32 { x * 2 }\npub fn doubled(x: u32) -> u32 { scale(x) }\n",
        )]);
        let scope = scope_of_with_examples(dir.path(), "doubled", &["doubled(3) == 6"], &[]);
        assert_eq!(scope.scope, "reached", "{:?}", scope.widened_because);
        assert_eq!(labels(&scope), vec!["scale"]);
    }

    /// The defect this module exists for, at unit scale: the helper a check
    /// runs is part of what the result stood on.
    #[test]
    fn a_plain_local_helper_is_in_the_scope_of_the_function_that_calls_it() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub fn scale(x: u32) -> u32 { x * 2 }\npub fn doubled(x: u32) -> u32 { scale(x) }\n",
        )]);
        let scope = scope_of(dir.path(), "doubled", &[]);
        assert_eq!(scope.scope, "reached", "{:?}", scope.widened_because);
        assert_eq!(
            labels(&scope),
            vec!["scale"],
            "the helper, and not the claimed function itself -- its own tokens are a hashed \
             input already, and hashing them twice would report one edit as two"
        );
    }

    /// `map(helper)` never writes `helper(..)`, and the body still runs.
    /// Following calls alone would miss it.
    #[test]
    fn a_helper_reached_only_as_a_value_is_in_scope_too() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub fn twice(x: u32) -> u32 { x + x }\npub fn apply(x: u32) -> u32 { let f = twice; \
             f(x) }\n",
        )]);
        let scope = scope_of(dir.path(), "apply", &[]);
        assert_eq!(scope.scope, "reached", "{:?}", scope.widened_because);
        assert!(
            labels(&scope).contains(&"twice".to_string()),
            "a function named as a value is a function that runs: {:?}",
            labels(&scope)
        );
    }

    /// A callee a proof replaces with a declared promise is not descended
    /// into, so its body is not what the result stood on -- the promise is,
    /// and the promise is hashed on its own. Hashing the body too would
    /// re-run every caller for an edit the proof never saw.
    #[test]
    fn a_callee_replaced_by_a_promise_is_not_in_scope() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub fn legacy(t: u8) -> u32 { if t == 0 { 1 } else { 2 } }\npub fn total(t: u8) -> \
             u32 { legacy(t) }\n",
        )]);
        // Declared for `legacy`, so `classify` reports it as assumed.
        let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        let mut declared = std::collections::BTreeMap::new();
        declared.insert(
            "legacy".to_string(),
            crate::callgraph::DeclaredContract {
                path: "legacy".into(),
                requires: vec![],
                ensures: vec!["|result| *result <= 10".into()],
            },
        );
        let mut resolver = Resolver::new(&lib, dir.path(), declared).unwrap();
        let first_party = scan_first_party(dir.path());
        let stubbed = BTreeSet::from(["legacy".to_string()]);
        let scope = code_scope(&mut resolver, &first_party, "total", &[], &stubbed);
        assert!(
            labels(&scope).is_empty(),
            "nothing but the claimed function itself is in reach: {:?}",
            labels(&scope)
        );
    }

    /// The failure direction that matters. An `impl` block means a method
    /// call or an operator can land in first-party code, and no syntactic
    /// walk can say which body. Ply widens instead of guessing.
    #[test]
    fn an_impl_block_anywhere_widens_the_scope_to_the_whole_crate() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub struct T;\nimpl T { pub fn go(&self) -> u32 { 1 } }\npub fn f(x: u32) -> u32 { x \
             }\n",
        )]);
        let scope = scope_of(dir.path(), "f", &[]);
        assert_eq!(
            scope.scope, "whole-crate",
            "an impl block puts bodies behind method calls Ply cannot follow"
        );
        assert!(
            scope
                .widened_because
                .as_deref()
                .is_some_and(|r| r.contains("impl")),
            "{:?}",
            scope.widened_because
        );
    }

    /// A contract is code: a helper named only inside `#[ply::ensures(..)]`
    /// runs on every generated case, and lives where no walk of the body
    /// would ever look.
    #[test]
    fn a_helper_named_only_in_a_contract_is_in_scope() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub fn expected(x: u32) -> u32 { x * 2 }
#[ply::ensures(|result| *result ==              expected(x))]
pub fn twice(x: u32) -> u32 { x + x }
",
        )]);
        let scope = scope_of(dir.path(), "twice", &[]);
        assert_eq!(scope.scope, "reached", "{:?}", scope.widened_because);
        assert_eq!(
            labels(&scope),
            vec!["expected"],
            "the oracle the contract calls is code the check runs"
        );
    }

    /// A helper in the *same module* as the claimed function is code the
    /// check runs, and editing it must re-earn the result.
    ///
    /// It did not. Resolution ran from the crate root, so a bare `helper(x)`
    /// written inside `mod maths` resolved to nothing, and "nothing" was
    /// read as "outside this workspace -- covered by the dependency
    /// versions instead". Editing `maths::helper` then left the caller's
    /// fingerprint identical and Ply carried a green forward over code a
    /// cold run reports as a violation. Reproduced end to end on
    /// 2026-09-05 before this test existed: cached `fuzzed(256) [reused]`,
    /// fresh `violation` at `x = 0`, same source.
    #[test]
    fn a_helper_in_the_claimed_functions_own_module_is_in_scope() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub mod maths {
    pub fn helper(x: u32) -> u32 { x }
    pub fn call(x: u32) -> u32 { helper(x) }
}
",
        )]);
        let scope = scope_of(dir.path(), "maths::call", &[]);
        assert_eq!(scope.scope, "reached", "{:?}", scope.widened_because);
        assert!(
            labels(&scope).iter().any(|l| l.ends_with("helper")),
            "the helper this function calls must be hashed, or editing it \
             leaves the caller's fingerprint unchanged: {:?}",
            labels(&scope)
        );
    }

    /// A type declaration is not a body, so no walk of bodies reaches it --
    /// but changing one changes what the bodies mean.
    ///
    /// `type Input = u8;` widened to `u16` leaves every function's tokens
    /// byte-identical while making `|result| *result <= 255` false. The
    /// walk allowed type aliases through its gate and then hashed only
    /// function bodies, so the change was invisible. Reproduced end to end
    /// on 2026-09-05: cached `fuzzed(256) [reused]`, fresh `violation` at
    /// `x = 256`, same source.
    #[test]
    fn a_type_declaration_is_hashed_even_though_no_body_reaches_it() {
        let alias = |ty: &str| {
            let dir = crate_with(&[(
                "src/lib.rs",
                &format!("pub type Input = {ty};\npub fn widen(x: Input) -> u32 {{ x as u32 }}\n"),
            )]);
            let scope = scope_of(dir.path(), "widen", &[]);
            assert_eq!(scope.scope, "reached", "{:?}", scope.widened_because);
            scope.units.clone()
        };
        assert_ne!(
            alias("u8"),
            alias("u16"),
            "widening the alias changes what `widen` accepts, so it must change \
             what the record hashes -- otherwise a green earned over `u8` is \
             carried forward onto `u16`"
        );
    }

    /// An attribute Ply does not recognise may be a macro that rewrites the
    /// body into something else entirely, and a walk of the tokens as
    /// written would be a walk of code that never runs.
    #[test]
    fn an_unrecognised_attribute_on_a_function_widens_the_scope() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "#[some_crate::instrument]
pub fn f(x: u32) -> u32 { x }
",
        )]);
        let scope = scope_of(dir.path(), "f", &[]);
        assert_eq!(scope.scope, "whole-crate", "{:?}", scope.widened_because);
    }

    /// A macro's expansion is not in the tokens the walk reads, so a call
    /// inside one is a call the walk cannot see.
    #[test]
    fn a_macro_in_a_reached_body_widens_the_scope() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub fn helper() -> bool { true }\npub fn f() -> u32 { assert!(helper()); 1 }\n",
        )]);
        let scope = scope_of(dir.path(), "f", &[]);
        assert_eq!(scope.scope, "whole-crate", "{:?}", scope.widened_because);
    }

    /// Ply writes a proof module into the crate it checks. Hashing it would
    /// make every run invalidate the results of the one before it, which is
    /// a cache that never hits.
    #[test]
    fn the_module_ply_writes_itself_is_never_part_of_the_hash() {
        let dir = crate_with(&[
            (
                "src/lib.rs",
                "pub struct T;\nimpl T {}\npub fn f() -> u32 { 1 }\nmod ply_generated;\n",
            ),
            ("src/ply_generated.rs", "pub fn proof_f() {}\n"),
        ]);
        let scope = scope_of(dir.path(), "f", &[]);
        assert_eq!(scope.scope, "whole-crate");
        let all = scope
            .units
            .iter()
            .map(|(l, t)| format!("{l}{t}"))
            .collect::<String>();
        assert!(
            !all.contains("ply_generated"),
            "the generated module must not be in the hash: {all}"
        );
    }

    /// Everything outside the workspace is covered by the compiler identity
    /// and the resolved dependency versions, not by hashing source Ply
    /// never had.
    #[test]
    fn a_call_out_of_the_workspace_is_not_hashed_as_source() {
        let dir = crate_with(&[(
            "src/lib.rs",
            "pub fn f(x: u32) -> u32 { std::cmp::max(x, 1) }\n",
        )]);
        let scope = scope_of(dir.path(), "f", &[]);
        assert_eq!(scope.scope, "reached", "{:?}", scope.widened_because);
        assert!(labels(&scope).is_empty(), "{:?}", labels(&scope));
    }

    /// A crate whose whole dependency set is inside the workspace has
    /// nothing outside it to pin, and says so in the same words whether or
    /// not a lockfile happens to exist -- otherwise the first run after a
    /// build would invalidate everything the run before it earned.
    #[test]
    fn a_crate_with_no_outside_dependencies_pins_the_same_thing_either_way() {
        let dir = crate_with(&[("src/lib.rs", "pub fn f() {}\n")]);
        let without = dependency_identity(dir.path());
        std::fs::write(
            dir.path().join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"c\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        assert_eq!(without, dependency_identity(dir.path()));
        assert_eq!(without, NO_EXTERNAL_CODE);
    }

    /// Two builds of the same package version from different git revisions
    /// are different code, and must not share an identity.
    ///
    /// The lockfile's `source =` line carries the revision (`...?branch=main#
    /// <sha>`), and it was read only as a boolean -- "is this external" --
    /// then discarded. Updating a git dependency without bumping its package
    /// version therefore left every fingerprint identical and every recorded
    /// green reusable over code that had changed underneath it. Reported by
    /// external review 2026-09-05; this is the reproduction it could not run.
    #[test]
    fn a_git_dependency_that_moved_is_not_the_same_dependency() {
        let lock = |rev: &str| {
            let dir = crate_with(&[("src/lib.rs", "pub fn f() {}\n")]);
            std::fs::write(
                dir.path().join("Cargo.lock"),
                format!(
                    r#"version = 4

[[package]]
name = "c"
version = "0.0.0"
dependencies = [
 "dep",
]

[[package]]
name = "dep"
version = "0.1.0"
source = "git+https://example.invalid/dep?branch=main#{rev}"
"#
                ),
            )
            .unwrap();
            dependency_identity(dir.path())
        };
        assert_ne!(
            lock("1111111111111111111111111111111111111111"),
            lock("2222222222222222222222222222222222222222"),
            "the same version at two different revisions is two different \
             dependencies, and a result earned against one must not be reused \
             against the other"
        );
    }

    /// When a lockfile holds two versions of one crate, the identity names
    /// the one this crate actually compiled against.
    ///
    /// Cargo writes `"dep 0.1.0"` in a `dependencies` list precisely when the
    /// bare name would be ambiguous. That version was being thrown away and
    /// packages were stored under their name alone, so the last `[[package]]`
    /// block in the file won and the fingerprint could name a version this
    /// crate never built with -- and would change when an unrelated crate
    /// bumped its own copy. Second half of the external review's third
    /// finding, 2026-09-05.
    #[test]
    fn two_versions_of_one_crate_do_not_overwrite_each_other() {
        let dir = crate_with(&[("src/lib.rs", "pub fn f() {}\n")]);
        std::fs::write(
            dir.path().join("Cargo.lock"),
            r#"version = 4

[[package]]
name = "c"
version = "0.0.0"
dependencies = [
 "dep 0.1.0",
]

[[package]]
name = "dep"
version = "0.1.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "dep"
version = "0.2.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
        )
        .unwrap();
        assert_eq!(
            dependency_identity(dir.path()),
            "dep 0.1.0",
            "the fingerprint must name the version this crate resolves to, not \
             whichever copy appeared last in the lockfile"
        );
    }

    /// The versions that are pinned are the ones this crate resolves to.
    /// The harness crate Ply generates depends on the target, never the
    /// reverse, so nothing it drags in can move the target's fingerprint.
    #[test]
    fn only_the_versions_this_crate_resolves_to_are_pinned() {
        let dir = crate_with(&[("src/lib.rs", "pub fn f() {}\n")]);
        std::fs::write(
            dir.path().join("Cargo.lock"),
            r#"version = 4

[[package]]
name = "c"
version = "0.0.0"
dependencies = [
 "serde",
]

[[package]]
name = "serde"
version = "1.0.9"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "c-ply-harness"
version = "0.0.0"
dependencies = [
 "c",
 "proptest",
]

[[package]]
name = "proptest"
version = "1.8.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
        )
        .unwrap();
        assert_eq!(dependency_identity(dir.path()), "serde 1.0.9");
    }

    #[test]
    fn a_crate_with_outside_dependencies_and_no_lockfile_says_so() {
        let dir = crate_with(&[("src/lib.rs", "pub fn f() {}\n")]);
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"c\"\nversion = \"0.0.0\"\n\n[dependencies]\nserde = \"1\"\n",
        )
        .unwrap();
        assert!(
            dependency_identity(dir.path()).contains("no Cargo.lock"),
            "without a lockfile Ply cannot know which versions an earlier run compiled against, \
             and must say that rather than pretend the set is empty"
        );
    }
}
