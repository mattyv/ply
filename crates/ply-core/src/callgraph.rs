//! Call-graph extraction and callee resolution for D5's third branch
//! (The-Ply-Spec.md §5.5, added 2026-08-25 after vetting 004).
//!
//! `bounded` is the only tier this matters for: Kani descends into a
//! callee's real body, so a caller's proof silently acquires the meaning of
//! every function it calls. proptest simply *runs* the callee, which is why
//! the fuzz tier crosses a legacy boundary happily and needs none of this.
//!
//! The rule §5.5 states is keyed on what the callee offers, so this module
//! answers exactly two questions about each call site in a contracted body:
//! *can Ply see the callee at all*, and *does any contract describe it*.
//!
//! Resolution follows the crate's own first-party structure: `use`
//! declarations (renames, nested groups and globs included), inline `mod`s,
//! file modules (`mod foo;` → `foo.rs` / `foo/mod.rs`), and the same walk
//! again inside a **path dependency's** `src/lib.rs`. A call whose path
//! leads out of the workspace — `std`, `core`, a registry crate — is
//! `Unresolved`: outside this rule's reach in v1, stated in §5.5 rather
//! than left for a user to discover.
//!
//! The two are not the same answer, and conflating them was a fail-open bug
//! (adversarial review of the post-004 fixes, D1, 2026-08-25): before
//! `use` declarations were read, `use rates::legacy_rate;` plus a bare-name
//! call classified `Unresolved`, and `Unresolved` meant *descend*, so the
//! most idiomatic spelling in Rust silently bought a clean `bounded(2)` over
//! an unclaimed body. The rule this module now holds to is one sentence:
//! **Ply descends only into a callee it resolved, or one that lies outside
//! the workspace entirely. First-party source Ply was pointed at and could
//! not read is refused (`Opaque`), never assumed harmless.**

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::visit::Visit;

/// One free-function call in a body: the path as written, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// The callee path exactly as the source spells it, `::`-joined
    /// (`fee_cents`, `ledger::fees::bps_for_tier`).
    pub path: String,
    pub line: usize,
    pub col: usize,
}

impl CallSite {
    /// `file.rs:41:9`-style location text for a diagnostic.
    pub fn where_text(&self) -> String {
        format!("line {}, column {}", self.line, self.col)
    }
}

/// Collects every *free-function* call (`f(..)`, `a::b::c(..)`) in a
/// function body, in source order, de-duplicated by path+position.
///
/// Method calls (`x.min(10_000)`, `v.len()`) are deliberately **not**
/// collected: they are receiver-dispatched, overwhelmingly `std`, and
/// flagging them would make the rule fire on every ordinary line of Rust
/// while telling a user nothing they could act on.
struct CallCollector {
    out: Vec<CallSite>,
}

impl<'ast> Visit<'ast> for CallCollector {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*node.func {
            let path = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            let span = p.path.segments[0].ident.span().start();
            self.out.push(CallSite {
                path,
                line: span.line,
                col: span.column + 1,
            });
        }
        syn::visit::visit_expr_call(self, node);
    }
}

pub fn call_sites(f: &syn::ItemFn) -> Vec<CallSite> {
    let mut c = CallCollector { out: Vec::new() };
    c.visit_block(&f.block);
    c.out
}

/// A contract declared for a callee in `ply.yaml` (§5.4's external-spec
/// route) — the mechanism that moves an unclaimed callee out of §5.5's
/// third branch and into its second.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredContract {
    /// The path as a caller spells it (`ledger::fees::bps_for_tier`).
    pub path: String,
    pub requires: Vec<String>,
    pub ensures: Vec<String>,
}

/// What Ply knows about one call site's callee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalleeStatus {
    /// The callee carries its own inline `#[ply::requires]`/`#[ply::ensures]`
    /// — D5's first two branches govern it, and Kani may descend.
    Contracted,
    /// No inline contract, but `ply.yaml` declares one: §5.5's second
    /// branch. The callee is stubbed and the caller's verdict is
    /// `conditional`.
    Assumed {
        contract: DeclaredContract,
        /// The one name Ply uses for this callee everywhere it says
        /// anything about it — the generated `#[kani::stub(..)]` attribute,
        /// the assumption text, the audit trust surface. Not necessarily the
        /// spelling at the call site, which may be a bare name that only a
        /// private `use` in `lib.rs` puts in scope and that the generated
        /// module therefore cannot name.
        canonical_path: String,
        signature: CalleeSignature,
    },
    /// Resolved, and nothing anywhere describes it: §5.5's third branch.
    /// Ply refuses to descend.
    Unclaimed,
    /// Ply was pointed at first-party source and could not read it: a
    /// `mod` whose file is missing, a path dependency whose `src/lib.rs`
    /// will not open or parse, or a bare name that could only have come
    /// from a glob import of one of those. The call is inside the
    /// workspace, so §5.5's reason for leaving `std` alone does not apply
    /// — and "I could not look" is not "there is nothing there", so this
    /// refuses rather than descends (§1's absence-of-evidence principle).
    /// The payload is the plain-language reason, for the diagnostic.
    Opaque(String),
    /// The call leads out of the workspace entirely (`std`, `core`, a
    /// registry crate) — no source Ply can read, and none it should expect
    /// to. Outside this rule's reach in v1: recorded in §5.5, not silently
    /// treated as claimed.
    Unresolved,
}

/// Enough of a resolved callee's signature to generate a Kani stub for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalleeSignature {
    /// `(name, rendered type source)` in declaration order.
    pub params: Vec<(String, String)>,
    /// The return type's source text; `None` for `-> ()`.
    pub return_type: Option<String>,
}

/// One classified call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCall {
    pub site: CallSite,
    pub status: CalleeStatus,
}

/// Everything resolution needs that is not the call itself: the caller's
/// own file, the crate directory (for path-dependency and file-module
/// lookup), and the contracts `ply.yaml` declares.
pub struct Resolver {
    local: SourceFile,
    crate_dir: PathBuf,
    declared: BTreeMap<String, DeclaredContract>,
    /// Parsed-file cache, keyed by path on disk. A `None` value records
    /// "Ply tried and could not read this", which is a different fact from
    /// "not cached yet" and must not be retried into a different answer.
    files: BTreeMap<PathBuf, Option<SourceFile>>,
    /// `[dependencies]` key → its crate root, or `None` when the key names
    /// no *path* dependency (a registry crate, or nothing at all).
    dep_roots: BTreeMap<String, Option<SourceFile>>,
}

/// One parsed source file plus the directory its own `mod x;` children are
/// looked for in: `src/` for `src/lib.rs`, `src/rates/` for `src/rates.rs`.
#[derive(Clone)]
struct SourceFile {
    ast: std::rc::Rc<syn::File>,
    dir: PathBuf,
}

/// A free function the resolver found, with everything a caller of the
/// resolver needs to talk about it.
pub struct FoundFn {
    pub item: syn::ItemFn,
    /// The path spelled from the crate root, with every `use` expanded:
    /// `legacy_rate` reached through `use rates::legacy_rate;` canonicalises
    /// to `rates::legacy_rate`. This is the string that lets the two halves
    /// of Ply agree about *which function* they are each talking about —
    /// the call site writes one spelling and `ply.yaml` writes another, and
    /// canonicalising both is what lets a promise attach to a callee.
    pub canonical: String,
    /// The parsed file the fn was declared in, so its `type` aliases are the
    /// ones read when the signature is interpreted.
    pub file: std::rc::Rc<syn::File>,
    /// `Some(reason)` when the fn is real but a generated harness sitting at
    /// the crate root could not *name* it: a private `mod` or a private `fn`
    /// somewhere below the crate root. Stated rather than discovered as a
    /// compile error in generated code.
    pub unnameable: Option<String>,
    /// Whether the whole walk stayed inside this crate. A resolution that
    /// went through a path dependency keeps the spelling the caller wrote,
    /// because the dependency's own crate-root path is not what a `#[kani::stub]`
    /// attribute in *this* crate can name.
    pub local: bool,
}

/// What one path lookup found. `Opaque` is the branch that keeps the rule
/// honest: it means Ply followed the path into first-party source and could
/// not read it, which is neither "found" nor "outside the workspace".
pub enum Resolution {
    Found(Box<FoundFn>),
    Opaque(String),
    NotFound,
}

impl Resolution {
    /// Records that the walk descended through module `name` to get here:
    /// the canonical path grows a segment on the front, and a private module
    /// anywhere in the chain makes the function unnameable from the crate
    /// root even though it was found.
    fn under_module(self, name: &str, module_is_private: bool) -> Resolution {
        match self {
            Resolution::Found(mut f) => {
                f.canonical = format!("{name}::{}", f.canonical);
                if module_is_private && f.unnameable.is_none() {
                    f.unnameable = Some(format!(
                        "the module `{name}` is private, so the harness Ply generates at the \
                         crate root cannot name anything inside it"
                    ));
                }
                Resolution::Found(f)
            }
            other => other,
        }
    }

    /// Records that the walk left this crate. The canonical path stops being
    /// something this crate's generated code can name, so callers fall back
    /// to the spelling at the call site.
    fn into_dependency(self) -> Resolution {
        match self {
            Resolution::Found(mut f) => {
                f.local = false;
                f.unnameable = None;
                Resolution::Found(f)
            }
            other => other,
        }
    }
}

/// How deep the resolver will follow module nesting and import chains
/// before giving up. This reader is not a compiler: a cycle a real crate
/// could not contain must still terminate here.
const MAX_DEPTH: usize = 8;

impl Resolver {
    /// `lib_src` is the caller's own file source; `declared` is keyed by the
    /// path a caller writes.
    pub fn new(
        lib_src: &str,
        crate_dir: &Path,
        declared: BTreeMap<String, DeclaredContract>,
    ) -> anyhow::Result<Resolver> {
        Ok(Resolver {
            local: SourceFile {
                ast: std::rc::Rc::new(syn::parse_file(lib_src)?),
                dir: crate_dir.join("src"),
            },
            crate_dir: crate_dir.to_path_buf(),
            declared,
            files: BTreeMap::new(),
            dep_roots: BTreeMap::new(),
        })
    }

    pub fn classify(&mut self, site: &CallSite) -> ResolvedCall {
        let status = self.status_of(&site.path);
        ResolvedCall {
            site: site.clone(),
            status,
        }
    }

    /// Resolves one path — written the way a `ply.yaml` claim writes it, or
    /// the way a call site writes it — to the function it names. The single
    /// entry point both halves of Ply use, so neither can have its own idea
    /// of where a function lives.
    pub fn lookup_fn(&mut self, path: &str) -> Resolution {
        let segments: Vec<String> = path.split("::").map(|s| s.to_string()).collect();
        self.resolve_path(&segments)
    }

    fn status_of(&mut self, path: &str) -> CalleeStatus {
        let found = match self.lookup_fn(path) {
            Resolution::Found(f) => *f,
            Resolution::Opaque(reason) => return CalleeStatus::Opaque(reason),
            Resolution::NotFound => return CalleeStatus::Unresolved,
        };
        if has_inline_contract(&found.item) {
            return CalleeStatus::Contracted;
        }
        // Two spellings name one function, and a promise must attach through
        // either. The path *as the caller spells it* is tried first, because
        // that is what a cross-crate `anchor:` produces; the canonical
        // crate-root path is tried second, because that is what a reader of
        // the crate writes in `ply.yaml` for a callee reached through a
        // `use` (`legacy_rate` at the call site, `rates::legacy_rate` in the
        // document). Before 2026-08-25 only the first was tried, so a
        // promise written for a function in a module attached to nothing.
        let contract = self
            .declared
            .get(path)
            .or_else(|| self.declared.get(&found.canonical));
        match contract {
            Some(contract) => CalleeStatus::Assumed {
                contract: contract.clone(),
                // The name a generated `#[kani::stub(..)]` must use. Inside
                // this crate that is the canonical path, which is nameable
                // from the generated module at the crate root; a bare name
                // that only exists because of a private `use` in lib.rs is
                // not. A resolution that crossed into a path dependency
                // keeps the caller's spelling, which is the one that names
                // the dependency here.
                canonical_path: if found.local {
                    found.canonical.clone()
                } else {
                    path.to_string()
                },
                signature: signature_of(&found.item),
            },
            None => CalleeStatus::Unclaimed,
        }
    }

    /// Every free function this crate declares that a claim could anchor to,
    /// as canonical crate-root paths, in a stable order. The item index
    /// behind `E0301`'s nearest-name suggestions — deliberately the *same*
    /// set [`Resolver::lookup_fn`] can find, so a suggestion is never a name
    /// that would then fail to resolve.
    pub fn fn_index(&mut self) -> Vec<String> {
        let local = self.local.clone();
        let mut out = Vec::new();
        self.collect_fns(&local, &mut Vec::new(), 0, &mut out);
        out
    }

    fn collect_fns(
        &mut self,
        file: &SourceFile,
        prefix: &mut Vec<String>,
        depth: usize,
        out: &mut Vec<String>,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for item in &file.ast.items.clone() {
            match item {
                syn::Item::Fn(f) => {
                    let mut segs = prefix.clone();
                    segs.push(f.sig.ident.to_string());
                    out.push(segs.join("::"));
                }
                syn::Item::Mod(m) => {
                    let name = m.ident.to_string();
                    let nested = match &m.content {
                        Some((_, inner)) => Some(SourceFile {
                            ast: std::rc::Rc::new(syn::File {
                                shebang: None,
                                attrs: vec![],
                                items: inner.clone(),
                            }),
                            dir: file.dir.join(&name),
                        }),
                        None => self.module_file(&file.dir, &name),
                    };
                    if let Some(nested) = nested {
                        prefix.push(name);
                        self.collect_fns(&nested, prefix, depth + 1, out);
                        prefix.pop();
                    }
                }
                _ => {}
            }
        }
    }

    fn resolve_path(&mut self, raw: &[String]) -> Resolution {
        let segs = expand_imports(&import_map(&self.local.ast), &strip_prefixes(raw));
        if segs.is_empty() {
            return Resolution::NotFound;
        }
        if segs.len() >= 2 {
            match self.dep_root(&segs[0]) {
                Some(Some(root)) => {
                    let in_dep = self.resolve_in_file(&root, &segs[1..], 0).into_dependency();
                    // Only `NotFound` falls through to the local file: a
                    // module of this crate may share a dependency's name, and
                    // in edition 2018+ the local one is what the path means.
                    // An `Opaque` from the dependency is a real answer and is
                    // not retried into a different one.
                    if !matches!(in_dep, Resolution::NotFound) {
                        return in_dep;
                    }
                }
                Some(None) => {
                    return Resolution::Opaque(format!(
                        "`{}` is a path dependency of this crate, but Ply could not read its \
                         `src/lib.rs`",
                        segs[0]
                    ));
                }
                None => {}
            }
        }
        let local = self.local.clone();
        let found = self.resolve_in_file(&local, &segs, 0);
        // A bare name that resolved nowhere may still have been brought into
        // scope by a glob. Names starting with a capital are Rust's
        // convention for a type or enum variant (`Some(x)`, `Ok(v)`,
        // `Wrapper(t)`) rather than a free function, and are left alone:
        // firing the boundary rule on `Some(x)` would tell a reader nothing
        // they could act on, which is the same reason method calls are not
        // call sites for this rule.
        if matches!(found, Resolution::NotFound) && segs.len() == 1 && !starts_uppercase(&segs[0]) {
            return self.resolve_through_globs(&segs[0]);
        }
        found
    }

    /// Walks one file for `segments`, following that file's own imports
    /// (which is how a re-export `pub use fees::bps_for_tier;` resolves),
    /// its inline `mod`s, and its file modules.
    fn resolve_in_file(
        &mut self,
        file: &SourceFile,
        segments: &[String],
        depth: usize,
    ) -> Resolution {
        if depth > MAX_DEPTH {
            return Resolution::NotFound;
        }
        let segs = expand_imports(&import_map(&file.ast), &strip_prefixes(segments));
        let Some((head, rest)) = segs.split_first() else {
            return Resolution::NotFound;
        };
        if rest.is_empty() {
            return match top_level_fn(&file.ast.items, head) {
                Some(f) => {
                    // A private item below the crate root cannot be *named*
                    // by the module Ply generates, which sits at the crate
                    // root: a sibling module sees only what is `pub` (or
                    // `pub(crate)`/`pub(super)`) further down. At the crate
                    // root itself, private is fine — the generated module is
                    // a child of the root and sees the root's own items.
                    let unnameable = (depth > 0
                        && matches!(f.vis, syn::Visibility::Inherited))
                    .then(|| {
                        format!(
                            "`{head}` is private to the module it is declared in, so the harness \
                             Ply generates at the crate root cannot call it by name"
                        )
                    });
                    Resolution::Found(Box::new(FoundFn {
                        item: f,
                        canonical: head.clone(),
                        file: file.ast.clone(),
                        unnameable,
                        local: true,
                    }))
                }
                None => Resolution::NotFound,
            };
        }
        // An inline `mod`: its items are right here.
        for item in &file.ast.items {
            if let syn::Item::Mod(m) = item
                && m.ident == head.as_str()
            {
                let mod_private = depth > 0 && matches!(m.vis, syn::Visibility::Inherited);
                let inner = match &m.content {
                    Some((_, inner)) => {
                        let nested = SourceFile {
                            ast: std::rc::Rc::new(syn::File {
                                shebang: None,
                                attrs: vec![],
                                items: inner.clone(),
                            }),
                            dir: file.dir.join(head),
                        };
                        self.resolve_in_file(&nested, rest, depth + 1)
                    }
                    // `mod foo;` — first-party code in another file.
                    None => match self.module_file(&file.dir, head) {
                        Some(sf) => self.resolve_in_file(&sf, rest, depth + 1),
                        None => Resolution::Opaque(format!(
                            "`{head}` is a module of this crate, but Ply could not read its source \
                             (it looked for `{head}.rs` and `{head}/mod.rs`)"
                        )),
                    },
                };
                return inner.under_module(head, mod_private);
            }
        }
        Resolution::NotFound
    }

    /// The deliberate rule for glob imports (`use rates::*;`). A glob whose
    /// source Ply *can* read is resolved exactly like a named import — the
    /// name is either in there or it is not. A glob into first-party source
    /// Ply cannot read leaves the bare name genuinely ambiguous, and an
    /// ambiguity inside the workspace is refused, never descended into. A
    /// glob into a crate outside the workspace (`use std::cmp::*;`) is left
    /// alone, for the same reason every other `std` call is: §5.5 says that
    /// gap out loud rather than pretending to have closed it.
    fn resolve_through_globs(&mut self, name: &str) -> Resolution {
        let prefixes = glob_prefixes(&self.local.ast);
        let mut opaque: Option<String> = None;
        for prefix in prefixes {
            let mut segs = prefix.clone();
            segs.push(name.to_string());
            let segs = strip_prefixes(&segs);
            if segs.len() < 2 {
                continue;
            }
            let r = match self.dep_root(&segs[0]) {
                Some(Some(root)) => self.resolve_in_file(&root, &segs[1..], 0),
                Some(None) => Resolution::Opaque(format!(
                    "`{}` is a path dependency of this crate, but Ply could not read its \
                     `src/lib.rs`",
                    segs[0]
                )),
                None => {
                    let local = self.local.clone();
                    self.resolve_in_file(&local, &segs, 0)
                }
            };
            match r {
                Resolution::Found(f) => return Resolution::Found(f),
                Resolution::Opaque(reason) => {
                    opaque.get_or_insert(format!(
                        "it may come from `use {}::*`, and {reason}",
                        prefix.join("::")
                    ));
                }
                Resolution::NotFound => {}
            }
        }
        match opaque {
            Some(reason) => Resolution::Opaque(reason),
            None => Resolution::NotFound,
        }
    }

    /// `Some(Some(root))` = a path dependency Ply read; `Some(None)` = a path
    /// dependency it could not read; `None` = not a path dependency at all
    /// (a registry crate, or not a dependency).
    fn dep_root(&mut self, dep_name: &str) -> Option<Option<SourceFile>> {
        if let Some(cached) = self.dep_roots.get(dep_name) {
            return Some(cached.clone());
        }
        let cargo_toml = std::fs::read_to_string(self.crate_dir.join("Cargo.toml")).ok()?;
        let rel = path_dependency(&cargo_toml, dep_name)?;
        let dep_src = self.crate_dir.join(rel).join("src");
        let root = self.read_file(&dep_src.join("lib.rs"), &dep_src);
        self.dep_roots.insert(dep_name.to_string(), root.clone());
        Some(root)
    }

    /// `mod foo;` in a file living in `dir`: `dir/foo.rs`, else
    /// `dir/foo/mod.rs`. Both are Rust's own conventions; a `#[path = ..]`
    /// attribute is not followed, and a module Ply cannot open is `Opaque`
    /// rather than silently absent.
    fn module_file(&mut self, dir: &Path, name: &str) -> Option<SourceFile> {
        let flat = dir.join(format!("{name}.rs"));
        if let Some(sf) = self.read_file(&flat, &dir.join(name)) {
            return Some(sf);
        }
        let nested_dir = dir.join(name);
        self.read_file(&nested_dir.join("mod.rs"), &nested_dir)
    }

    fn read_file(&mut self, path: &Path, child_dir: &Path) -> Option<SourceFile> {
        if let Some(cached) = self.files.get(path) {
            return cached.clone();
        }
        let parsed = std::fs::read_to_string(path)
            .ok()
            .and_then(|src| syn::parse_file(&src).ok())
            .map(|ast| SourceFile {
                ast: std::rc::Rc::new(ast),
                dir: child_dir.to_path_buf(),
            });
        self.files.insert(path.to_path_buf(), parsed.clone());
        parsed
    }
}

/// `self::f` and `crate::f` name the caller's own crate root; neither adds
/// anything to resolve.
fn strip_prefixes(segments: &[String]) -> Vec<String> {
    segments
        .iter()
        .skip_while(|s| s.as_str() == "self" || s.as_str() == "crate")
        .cloned()
        .collect()
}

fn starts_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}

fn top_level_fn(items: &[syn::Item], name: &str) -> Option<syn::ItemFn> {
    items.iter().find_map(|item| match item {
        syn::Item::Fn(f) if f.sig.ident == name => Some(f.clone()),
        _ => None,
    })
}

/// Every name a file's `use` declarations bind, mapped to the path they
/// bind it to: `use rates::legacy_rate;` → `legacy_rate` → `rates,
/// legacy_rate`; `use a::{b, c::d as e};` → `b` → `a,b` and `e` → `a,c,d`.
/// Globs bind no name and are collected separately (`glob_prefixes`).
fn import_map(file: &syn::File) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for item in &file.items {
        if let syn::Item::Use(u) = item {
            walk_use_tree(&u.tree, &mut Vec::new(), &mut out);
        }
    }
    out
}

fn walk_use_tree(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            walk_use_tree(&p.tree, prefix, out);
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
            // Keyed on the *local* name: `cap_bps as capped` is called
            // `capped` at the call site, and that is the only spelling a
            // reader of the body ever sees.
            out.insert(r.rename.to_string(), full);
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                walk_use_tree(item, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// The prefix of every `use ...::*;` in a file.
fn glob_prefixes(file: &syn::File) -> Vec<Vec<String>> {
    fn walk(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        match tree {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                walk(&p.tree, prefix, out);
                prefix.pop();
            }
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    walk(item, prefix, out);
                }
            }
            syn::UseTree::Glob(_) => out.push(prefix.clone()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for item in &file.items {
        if let syn::Item::Use(u) = item {
            walk(&u.tree, &mut Vec::new(), &mut out);
        }
    }
    out
}

/// Rewrites a path's first segment through the imports in scope, repeatedly
/// (`use a::b;` plus `use b::c;` makes `c` mean `a::b::c`), stopping as soon
/// as nothing changes or the chain gets implausibly long.
fn expand_imports(imports: &BTreeMap<String, Vec<String>>, segments: &[String]) -> Vec<String> {
    let mut segs = segments.to_vec();
    for _ in 0..MAX_DEPTH {
        let Some((head, rest)) = segs.split_first() else {
            return segs;
        };
        let Some(target) = imports.get(head) else {
            return segs;
        };
        if target == &segs || (target.len() == 1 && &target[0] == head) {
            return segs;
        }
        let mut next = target.clone();
        next.extend_from_slice(rest);
        if next == segs {
            return segs;
        }
        segs = next;
    }
    segs
}

/// Finds `path = "..."` for one `[dependencies]` entry, by line scanning —
/// the same deliberately narrow convention as
/// `harness_crate::read_crate_names`, not a general TOML reader. Handles
/// the inline-table form Cargo path dependencies always take
/// (`name = { package = "...", path = "..." }`), single or multi-line.
pub fn path_dependency(cargo_toml: &str, dep_name: &str) -> Option<String> {
    let mut in_deps = false;
    let mut pending = false;
    for line in cargo_toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_deps = t == "[dependencies]" || t == "[dev-dependencies]";
            pending = false;
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some(rest) = t.strip_prefix(dep_name) {
            let rest = rest.trim_start();
            if rest.starts_with('=') || rest.starts_with('.') {
                pending = true;
                if let Some(p) = extract_quoted_value(t, "path") {
                    return Some(p);
                }
                continue;
            }
        }
        if pending {
            if let Some(p) = extract_quoted_value(t, "path") {
                return Some(p);
            }
            if t.contains('}') || t.is_empty() {
                pending = false;
            }
        }
    }
    None
}

fn extract_quoted_value(line: &str, key: &str) -> Option<String> {
    let idx = line.find(key)?;
    let rest = line[idx + key.len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Walks `mod` items for all but the last segment, then looks for a
/// top-level `fn` with the last segment's name. Test-only since the
/// resolver gained real module/import following: production resolution
/// goes through `Resolver::resolve_in_file`, which also reads file modules
/// and `use` declarations and can answer `Opaque`.
#[cfg(test)]
fn find_fn(file: &syn::File, segments: &[&str]) -> Option<syn::ItemFn> {
    fn walk(items: &[syn::Item], segments: &[&str]) -> Option<syn::ItemFn> {
        let (head, rest) = segments.split_first()?;
        if rest.is_empty() {
            for item in items {
                if let syn::Item::Fn(f) = item
                    && f.sig.ident == head
                {
                    return Some(f.clone());
                }
            }
            return None;
        }
        for item in items {
            if let syn::Item::Mod(m) = item
                && m.ident == head
                && let Some((_, inner)) = &m.content
            {
                return walk(inner, rest);
            }
        }
        None
    }
    // `self::f` and `crate::f` name the caller's own file.
    let segments: Vec<&str> = segments
        .iter()
        .copied()
        .skip_while(|s| *s == "self" || *s == "crate")
        .collect();
    walk(&file.items, &segments)
}

fn has_inline_contract(f: &syn::ItemFn) -> bool {
    f.attrs.iter().any(|attr| {
        let segs: Vec<String> = attr
            .path()
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        segs == ["ply", "requires"] || segs == ["ply", "ensures"]
    })
}

fn signature_of(f: &syn::ItemFn) -> CalleeSignature {
    let mut params = Vec::new();
    for arg in &f.sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            let name = match &*pt.pat {
                syn::Pat::Ident(pi) => pi.ident.to_string(),
                other => other.to_token_stream().to_string(),
            };
            params.push((name, pt.ty.to_token_stream().to_string()));
        }
    }
    let return_type = match &f.sig.output {
        syn::ReturnType::Default => None,
        syn::ReturnType::Type(_, ty) => Some(ty.to_token_stream().to_string()),
    };
    CalleeSignature {
        params,
        return_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fn(src: &str, name: &str) -> syn::ItemFn {
        let file = syn::parse_file(src).unwrap();
        find_fn(&file, &[name]).unwrap()
    }

    #[test]
    fn collects_free_calls_and_ignores_method_calls() {
        let f = parse_fn(
            r#"
pub fn caller(x: u32) -> u32 {
    let bps = ledger::fees::bps_for_tier(3).min(10_000);
    helper(x, bps)
}
"#,
            "caller",
        );
        let sites = call_sites(&f);
        let paths: Vec<&str> = sites.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["ledger::fees::bps_for_tier", "helper"],
            "`.min(..)` is a method call on a receiver, not a free-function call: {sites:?}"
        );
        assert_eq!(sites[0].line, 3, "the call site's own line is reported");
    }

    #[test]
    fn a_callee_with_an_inline_contract_is_contracted() {
        let src = r#"
#[ply::ensures(|result| *result > 0)]
pub fn helper(x: u32) -> u32 { x + 1 }
pub fn caller(x: u32) -> u32 { helper(x) }
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(r.status_of("helper"), CalleeStatus::Contracted);
    }

    #[test]
    fn a_callee_with_no_contract_anywhere_is_unclaimed() {
        let src = r#"
pub fn helper(x: u32) -> u32 { x + 1 }
pub fn caller(x: u32) -> u32 { helper(x) }
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(r.status_of("helper"), CalleeStatus::Unclaimed);
    }

    #[test]
    fn a_ply_yaml_declared_contract_moves_a_callee_to_assumed_with_its_signature() {
        let src = r#"
pub fn helper(tier: u8) -> u32 { 150 }
"#;
        let mut declared = BTreeMap::new();
        declared.insert(
            "helper".to_string(),
            DeclaredContract {
                path: "helper".into(),
                requires: vec![],
                ensures: vec!["|result| *result <= 10_000".into()],
            },
        );
        let mut r = Resolver::new(src, Path::new("."), declared).unwrap();
        match r.status_of("helper") {
            CalleeStatus::Assumed {
                contract,
                canonical_path,
                signature,
            } => {
                assert_eq!(canonical_path, "helper");
                assert_eq!(contract.ensures, vec!["|result| *result <= 10_000"]);
                assert_eq!(
                    signature.params,
                    vec![("tier".to_string(), "u8".to_string())]
                );
                assert_eq!(signature.return_type.as_deref(), Some("u32"));
            }
            other => panic!("expected Assumed, got {other:?}"),
        }
    }

    // --- the `use`-import hole (adversarial review of the post-004 fixes,
    // D1): the boundary rule keys on what a callee *is*, never on how the
    // call is spelled. Every spelling below resolved to `Unresolved` before
    // 2026-08-25, and `Unresolved` meant descend.

    #[test]
    fn a_use_imported_callee_is_classified_exactly_like_a_qualified_one() {
        let src = r#"
mod rates {
    pub fn legacy_rate(tier: u8) -> u32 { 150 }
}
use rates::legacy_rate;
pub fn caller(t: u8) -> u32 { legacy_rate(t) }
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(
            r.status_of("legacy_rate"),
            CalleeStatus::Unclaimed,
            "`use rates::legacy_rate;` plus a bare-name call is the most ordinary spelling in \
             Rust, and it must not buy a descent into an unclaimed body"
        );
    }

    #[test]
    fn a_renamed_import_is_followed_to_the_function_it_names() {
        let src = r#"
mod rates {
    pub fn cap_bps(b: u32) -> u32 { b }
}
use rates::cap_bps as capped;
pub fn caller(b: u32) -> u32 { capped(b) }
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(r.status_of("capped"), CalleeStatus::Unclaimed);
    }

    #[test]
    fn a_nested_use_group_binds_every_name_it_lists() {
        let src = r#"
mod rates {
    pub fn a() -> u32 { 1 }
    pub mod inner { pub fn b() -> u32 { 2 } }
}
use rates::{a, inner::b as bee};
pub fn caller() -> u32 { a() + bee() }
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(r.status_of("a"), CalleeStatus::Unclaimed);
        assert_eq!(r.status_of("bee"), CalleeStatus::Unclaimed);
    }

    #[test]
    fn an_imported_module_prefix_is_followed_too() {
        // `use ledger::fees;` then `fees::bps_for_tier(..)`: the head
        // segment is the import, not the fn.
        let src = r#"
mod ledger { pub mod fees { pub fn bps_for_tier(t: u8) -> u32 { 150 } } }
use ledger::fees;
pub fn caller(t: u8) -> u32 { fees::bps_for_tier(t) }
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(r.status_of("fees::bps_for_tier"), CalleeStatus::Unclaimed);
    }

    #[test]
    fn a_glob_ply_can_see_through_is_resolved_exactly_like_a_named_import() {
        let src = r#"
mod rates { pub fn legacy_rate(t: u8) -> u32 { 150 } }
use rates::*;
pub fn caller(t: u8) -> u32 { legacy_rate(t) }
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(
            r.status_of("legacy_rate"),
            CalleeStatus::Unclaimed,
            "the name is either in the glob's module or it is not, and Ply can read this one"
        );
    }

    #[test]
    fn a_glob_over_a_module_ply_cannot_read_refuses_rather_than_descends() {
        // `mod legacy;` with no file behind it: first-party code Ply was
        // pointed at and could not open. The bare name might be from there,
        // and "I could not look" is not "there is nothing there".
        let src = r#"
mod legacy;
use legacy::*;
pub fn caller(t: u8) -> u32 { legacy_rate(t) }
"#;
        let mut r = Resolver::new(src, Path::new("/nonexistent"), BTreeMap::new()).unwrap();
        match r.status_of("legacy_rate") {
            CalleeStatus::Opaque(reason) => {
                assert!(
                    reason.contains("legacy"),
                    "the reason must name the module a reader has to go look at: {reason}"
                );
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_glob_over_a_crate_outside_the_workspace_is_left_alone() {
        // `use std::cmp::*;` is the same gap §5.5 already states for every
        // other `std` call. Refusing here would fire on ordinary Rust and
        // tell the reader nothing they could act on.
        let src = r#"
use std::cmp::*;
pub fn caller(a: u32, b: u32) -> u32 { max(a, b) }
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(r.status_of("max"), CalleeStatus::Unresolved);
    }

    #[test]
    fn a_tuple_variant_constructor_is_not_a_boundary_call() {
        // `Some(x)`/`Ok(v)` are `ExprCall`s with a one-segment path. Under a
        // glob Ply cannot see through they would otherwise become refusals,
        // on every ordinary line of Rust.
        let src = r#"
mod legacy;
use legacy::*;
pub fn caller(x: u32) -> Option<u32> { Some(x) }
"#;
        let mut r = Resolver::new(src, Path::new("/nonexistent"), BTreeMap::new()).unwrap();
        assert_eq!(r.status_of("Some"), CalleeStatus::Unresolved);
    }

    #[test]
    fn a_file_module_ply_cannot_read_is_refused_never_silently_skipped() {
        let src = r#"
mod legacy;
pub fn caller(t: u8) -> u32 { legacy::legacy_rate(t) }
"#;
        let mut r = Resolver::new(src, Path::new("/nonexistent"), BTreeMap::new()).unwrap();
        assert!(
            matches!(r.status_of("legacy::legacy_rate"), CalleeStatus::Opaque(_)),
            "a module of this crate that Ply could not open is not the same fact as a call into std"
        );
    }

    #[test]
    fn a_file_module_on_disk_is_read_and_its_fn_classified() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/rates.rs"),
            "pub fn legacy_rate(t: u8) -> u32 { 150 }\n",
        )
        .unwrap();
        let src = r#"
mod rates;
use rates::legacy_rate;
pub fn caller(t: u8) -> u32 { legacy_rate(t) }
"#;
        let mut r = Resolver::new(src, dir.path(), BTreeMap::new()).unwrap();
        assert_eq!(r.status_of("legacy_rate"), CalleeStatus::Unclaimed);
    }

    #[test]
    fn a_std_call_is_unresolved_not_unclaimed() {
        let src = "pub fn caller(x: u32) -> u64 { u64::from(x) }";
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(
            r.status_of("u64::from"),
            CalleeStatus::Unresolved,
            "Ply cannot see std's source, and must not claim it decided anything about it"
        );
    }

    #[test]
    fn reads_a_renamed_path_dependency_from_cargo_toml() {
        let toml = r#"
[dependencies]
ply = { package = "ply-attrs", path = "../../../crates/ply-attrs" }
ledger = { package = "ply-vetting-004-ledger", path = "../legacy" }
"#;
        assert_eq!(path_dependency(toml, "ledger").unwrap(), "../legacy");
        assert_eq!(path_dependency(toml, "nope"), None);
    }

    #[test]
    fn finds_a_fn_nested_in_an_inline_module() {
        let file = syn::parse_file(
            r#"
pub mod fees {
    pub fn bps_for_tier(tier: u8) -> u32 { 150 }
}
"#,
        )
        .unwrap();
        assert!(find_fn(&file, &["fees", "bps_for_tier"]).is_some());
        assert!(find_fn(&file, &["fees", "missing"]).is_none());
    }
}
