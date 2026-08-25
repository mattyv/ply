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
fn item_is_walkable(item: &syn::Item, label: &str) -> Result<(), String> {
    let named = |what: &str, name: String| {
        Err(format!(
            "{label} declares {what} `{name}`, and Ply's call walk cannot tell which of its \
             bodies a method call or an operator would run"
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
        syn::Item::Impl(i) => named(
            "an `impl` block for",
            i.self_ty.to_token_stream().to_string(),
        ),
        syn::Item::Trait(t) => named("a trait", t.ident.to_string()),
        syn::Item::Const(c) => named("a constant", c.ident.to_string()),
        syn::Item::Static(s) => named("a static", s.ident.to_string()),
        syn::Item::Macro(m) => Err(format!(
            "{label} declares or invokes a macro at the top level, and a macro's expansion is \
             not in the tokens Ply's call walk reads: {}",
            m.mac.path.to_token_stream()
        )),
        other => Err(format!(
            "{label} declares `{}`, an item kind Ply's call walk does not know how to bound",
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
            item.to_tokens(&mut tokens);
        }
        units.push((label, tokens.to_string()));
    }
    units.sort();
    FirstParty { units, gate }
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
    stubbed: &BTreeSet<String>,
) -> CodeScope {
    if let Some(reason) = &first_party.gate {
        return widened(first_party, reason.clone());
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut units: Vec<(String, String)> = Vec::new();
    queue.push_back(root_fn_path.to_string());
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
        for path in mentions.paths {
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
            in_deps = t == "[dependencies]"
                || t == "[dev-dependencies]"
                || t.starts_with("[dependencies.")
                || t.starts_with("[dev-dependencies.");
            pending = t
                .strip_prefix("[dependencies.")
                .or(t.strip_prefix("[dev-dependencies."))
                .map(|rest| rest.trim_end_matches(']').to_string());
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
            if t.starts_with("[dependencies.") || t.starts_with("[dev-dependencies.") {
                count += 1;
                in_deps = false;
                continue;
            }
            in_deps = t == "[dependencies]" || t == "[dev-dependencies]";
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
fn registry_packages_reachable_from(lock: &str, root: &str) -> Vec<String> {
    struct Pkg {
        version: String,
        external: bool,
        deps: Vec<String>,
    }
    let mut packages: std::collections::BTreeMap<String, Pkg> = std::collections::BTreeMap::new();
    let mut name = String::new();
    let mut version = String::new();
    let mut external = false;
    let mut deps: Vec<String> = Vec::new();
    let mut in_deps_list = false;
    let mut started = false;
    let flush = |name: &mut String,
                 version: &mut String,
                 external: &mut bool,
                 deps: &mut Vec<String>,
                 packages: &mut std::collections::BTreeMap<String, Pkg>| {
        if !name.is_empty() {
            packages.insert(
                std::mem::take(name),
                Pkg {
                    version: std::mem::take(version),
                    external: std::mem::replace(external, false),
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
                &mut external,
                &mut deps,
                &mut packages,
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
            let entry = t.trim_matches(|c| c == '"' || c == ',');
            if let Some(first) = entry.split_whitespace().next()
                && !first.is_empty()
            {
                deps.push(first.trim_matches('"').to_string());
            }
            continue;
        }
        if let Some(v) = t.strip_prefix("name = ") {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = t.strip_prefix("version = ") {
            version = v.trim_matches('"').to_string();
        } else if t.starts_with("source = ") {
            external = true;
        } else if t.starts_with("dependencies = [") {
            in_deps_list = !t.ends_with(']');
        }
    }
    flush(
        &mut name,
        &mut version,
        &mut external,
        &mut deps,
        &mut packages,
    );

    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(root.to_string());
    while let Some(next) = queue.pop_front() {
        if !seen.insert(next.clone()) {
            continue;
        }
        let Some(pkg) = packages.get(&next) else {
            continue;
        };
        if pkg.external {
            out.insert(format!("{next} {}", pkg.version));
        }
        for d in &pkg.deps {
            queue.push_back(d.clone());
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
        let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        let mut resolver = Resolver::new(&lib, dir, Default::default()).unwrap();
        let first_party = scan_first_party(dir);
        let stubbed: BTreeSet<String> = stubbed.iter().map(|s| s.to_string()).collect();
        code_scope(&mut resolver, &first_party, root, &stubbed)
    }

    fn labels(scope: &CodeScope) -> Vec<String> {
        scope.units.iter().map(|(l, _)| l.clone()).collect()
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
        let scope = code_scope(&mut resolver, &first_party, "total", &stubbed);
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
