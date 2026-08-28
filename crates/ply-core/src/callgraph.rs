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
use syn::spanned::Spanned;
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
    /// The source file on disk. Inline modules inherit this unchanged.
    path: PathBuf,
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
    /// Exact, zero-based range of the complete function item in its source
    /// file. The path is relative to the crate being verified.
    pub source_span: crate::diag::Span,
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
    /// Whether this is `Type::method` rather than a free function (added
    /// 2026-08-27, method resolution). Generated-harness codegen calls a
    /// free function by its bare final segment (after `use`-importing the
    /// whole path); it cannot do that for a method, which is not itself an
    /// importable item -- `use crate::Bucket::new;` does not compile. Code
    /// that builds a call expression from `canonical` needs to know which
    /// shape it has: a bare identifier, or `Type::method` reached by
    /// importing only `Type`.
    pub is_method: bool,
}

/// What one path lookup found. `Opaque` is the branch that keeps the rule
/// honest: it means Ply followed the path into first-party source and could
/// not read it, which is neither "found" nor "outside the workspace".
pub enum Resolution {
    Found(Box<FoundFn>),
    Opaque(String),
    NotFound,
    /// `Type::method` named something real -- a method with a receiver, an
    /// item in a generic `impl` block, or a trait's own method (a bare
    /// signature, a default body, or a trait-impl override) -- that this
    /// slice's scope refuses to check, by name, rather than either guessing
    /// at it or reporting the false "no such function" (added 2026-08-27,
    /// method resolution). The payload is the plain-language reason.
    Refused(String),
    /// `Type::method` matched more than one candidate in the same file --
    /// two `impl` blocks for the same type each defining a same-named
    /// method (real Rust: e.g. two concrete instantiations of a generic
    /// type, `impl Foo<u8>` and `impl Foo<u16>`, each with their own `bar`).
    /// Ply will not pick one; the payload names why it could not.
    Ambiguous(String),
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
                path: crate_dir.join("src/lib.rs"),
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

    /// The names of every field a `struct` declares as *not* fully `pub`
    /// (`pub(crate)`/`pub(super)`/private all count, since none of them are
    /// visible from a harness crate outside this one) -- what the "a `Self`
    /// answer is always fine" rule (§5.4b) needs to check before it ever
    /// applies on the sampling tier (adversarial review, 2026-08-27): the
    /// exhaustive tier's harness lives *inside* this crate and sees a
    /// private field fine; the fuzz/test tier's harness is a separate crate
    /// and cannot. `None` when `type_path` does not resolve to a `struct`
    /// at all (an enum, or nothing) -- there is nothing this check applies
    /// to either way.
    pub fn private_field_names(&mut self, type_path: &str) -> Option<Vec<String>> {
        let segments: Vec<String> = type_path.split("::").map(|s| s.to_string()).collect();
        let (_, name, file) = self.resolve_type_decl(&segments)?;
        file.ast.items.iter().find_map(|item| {
            let syn::Item::Struct(s) = item else {
                return None;
            };
            if s.ident != name {
                return None;
            }
            Some(
                s.fields
                    .iter()
                    .filter(|f| !matches!(f.vis, syn::Visibility::Public(_)))
                    .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
                    .collect(),
            )
        })
    }

    fn status_of(&mut self, path: &str) -> CalleeStatus {
        let found = match self.lookup_fn(path) {
            Resolution::Found(f) => *f,
            Resolution::Opaque(reason) => return CalleeStatus::Opaque(reason),
            Resolution::NotFound => return CalleeStatus::Unresolved,
            // A call site naming a method Ply refuses to check (a receiver,
            // a generic impl, a trait method): real and resolved, same as
            // §5.5's third branch means for a free function -- nothing
            // vouches for it, so Kani must not descend. `Unclaimed` is
            // exactly that branch, not a new one.
            Resolution::Refused(_) => return CalleeStatus::Unclaimed,
            // Ambiguous is a stronger fact than "nothing vouches for it" --
            // Ply does not even know which function this is, so it is
            // refused the same way an unreadable module is: "I could not
            // resolve this with confidence" is not "there is nothing here".
            Resolution::Ambiguous(reason) => return CalleeStatus::Opaque(reason),
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
        for item in &file.ast.items {
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
                            path: file.path.clone(),
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

    /// Every module this crate declares, paired with the file at that
    /// position and whether *any* module on the chain down from the crate
    /// root is non-`pub` (the crate root itself always carries `false`) --
    /// the answer `resolve_method_globally` needs to check every `impl`
    /// block in the crate against a type's declaration, not just whichever
    /// one happens to sit in the same file. Same traversal shape as
    /// [`Resolver::collect_fns`], a different payload.
    fn all_modules(&mut self) -> Vec<(Vec<String>, SourceFile, bool)> {
        let local = self.local.clone();
        let mut out = vec![(Vec::new(), local.clone(), false)];
        self.collect_modules(&local, &mut Vec::new(), false, 0, &mut out);
        out
    }

    fn collect_modules(
        &mut self,
        file: &SourceFile,
        prefix: &mut Vec<String>,
        chain_private: bool,
        depth: usize,
        out: &mut Vec<(Vec<String>, SourceFile, bool)>,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for item in &file.ast.items {
            if let syn::Item::Mod(m) = item {
                let name = m.ident.to_string();
                let this_private = chain_private || matches!(m.vis, syn::Visibility::Inherited);
                let nested = match &m.content {
                    Some((_, inner)) => Some(SourceFile {
                        ast: std::rc::Rc::new(syn::File {
                            shebang: None,
                            attrs: vec![],
                            items: inner.clone(),
                        }),
                        dir: file.dir.join(&name),
                        path: file.path.clone(),
                    }),
                    None => self.module_file(&file.dir, &name),
                };
                if let Some(nested) = nested {
                    prefix.push(name);
                    out.push((prefix.clone(), nested.clone(), this_private));
                    self.collect_modules(&nested, prefix, this_private, depth + 1, out);
                    prefix.pop();
                }
            }
        }
    }

    /// Resolves a *type* path (a `struct`/`enum`, never a fn) to the module
    /// path its declaration genuinely sits at -- walking `use` imports
    /// (renames and re-exports included, exactly as a free function's path
    /// already does), inline `mod`s and file modules. This is the ground
    /// truth a `Type::method` claim's type half is checked against: never
    /// the module an `impl` block for it happens to be written in, never a
    /// re-export's own location, only where the `struct`/`enum` itself is
    /// declared.
    fn resolve_type_decl(
        &mut self,
        segments: &[String],
    ) -> Option<(Vec<String>, String, SourceFile)> {
        let local = self.local.clone();
        self.type_decl_in_file(&local, Vec::new(), segments, 0)
    }

    fn type_decl_in_file(
        &mut self,
        file: &SourceFile,
        module_path: Vec<String>,
        segments: &[String],
        depth: usize,
    ) -> Option<(Vec<String>, String, SourceFile)> {
        if depth > MAX_DEPTH {
            return None;
        }
        let raw = strip_prefixes(segments);
        let segs = expand_imports(&import_map(&file.ast), &raw);
        let (head, rest) = segs.split_first()?;
        if rest.is_empty() {
            return type_item_ident(&file.ast.items, head)
                .map(|name| (module_path, name, file.clone()));
        }
        for item in &file.ast.items {
            if let syn::Item::Mod(m) = item
                && m.ident == head.as_str()
            {
                let inner = match &m.content {
                    Some((_, inner)) => SourceFile {
                        ast: std::rc::Rc::new(syn::File {
                            shebang: None,
                            attrs: vec![],
                            items: inner.clone(),
                        }),
                        dir: file.dir.join(head),
                        path: file.path.clone(),
                    },
                    None => self.module_file(&file.dir, head)?,
                };
                let mut new_path = module_path.clone();
                new_path.push(head.clone());
                return self.type_decl_in_file(&inner, new_path, rest, depth + 1);
            }
        }
        None
    }

    /// The same walk as [`Resolver::resolve_type_decl`], but for a `trait`
    /// item instead of a `struct`/`enum` -- used only when the type half of
    /// a `Type::method` claim names no real type at all, so it may still be
    /// a trait declaration (`Widget::size` naming `trait Widget`'s own
    /// method, not an `impl`).
    fn resolve_trait_decl(&mut self, segments: &[String]) -> Option<(Vec<String>, SourceFile)> {
        let local = self.local.clone();
        self.trait_decl_in_file(&local, Vec::new(), segments, 0)
    }

    fn trait_decl_in_file(
        &mut self,
        file: &SourceFile,
        module_path: Vec<String>,
        segments: &[String],
        depth: usize,
    ) -> Option<(Vec<String>, SourceFile)> {
        if depth > MAX_DEPTH {
            return None;
        }
        let raw = strip_prefixes(segments);
        let segs = expand_imports(&import_map(&file.ast), &raw);
        let (head, rest) = segs.split_first()?;
        if rest.is_empty() {
            return trait_item_ident(&file.ast.items, head).map(|_| (module_path, file.clone()));
        }
        for item in &file.ast.items {
            if let syn::Item::Mod(m) = item
                && m.ident == head.as_str()
            {
                let inner = match &m.content {
                    Some((_, inner)) => SourceFile {
                        ast: std::rc::Rc::new(syn::File {
                            shebang: None,
                            attrs: vec![],
                            items: inner.clone(),
                        }),
                        dir: file.dir.join(head),
                        path: file.path.clone(),
                    },
                    None => self.module_file(&file.dir, head)?,
                };
                let mut new_path = module_path.clone();
                new_path.push(head.clone());
                return self.trait_decl_in_file(&inner, new_path, rest, depth + 1);
            }
        }
        None
    }

    /// Resolves an `impl` block's own `self_ty`, from *that block's own*
    /// module position and file, to the module path + name it actually
    /// names -- respecting `super`/`crate`/`self` qualification and that
    /// file's own `use` imports, never assumed to be "whatever this file's
    /// module happens to be". This is what lets Ply tell `impl super::Root`
    /// (written inside a submodule, naming the crate root's `Root`) apart
    /// from an `impl Root` in that very same file (naming a `Root` declared
    /// in the submodule itself) -- two syntactically similar blocks naming
    /// two different types, which is exactly the shape the false pass this
    /// replaces exploited.
    fn resolve_self_ty_target(
        &self,
        from_module: &[String],
        from_file: &SourceFile,
        self_ty: &syn::Type,
    ) -> Option<(Vec<String>, String)> {
        let syn::Type::Path(p) = self_ty else {
            return None;
        };
        let segs: Vec<String> = p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        let (name, mods) = segs.split_last()?;
        let name = name.clone();
        if mods.first().map(String::as_str) == Some("crate") {
            return Some((mods[1..].to_vec(), name));
        }
        if mods.first().map(String::as_str) == Some("self") {
            let mut full = from_module.to_vec();
            full.extend(mods[1..].iter().cloned());
            return Some((full, name));
        }
        if mods.first().map(String::as_str) == Some("super") {
            let up = mods.iter().take_while(|s| s.as_str() == "super").count();
            if up > from_module.len() {
                return None;
            }
            let mut full = from_module[..from_module.len() - up].to_vec();
            full.extend(mods[up..].iter().cloned());
            return Some((full, name));
        }
        // Bare (no explicit qualifier): follow this file's own `use`
        // imports first -- `expand_imports` already resolves a rename or a
        // re-export to an absolute path from the crate root, exactly as it
        // does for a free function's own call sites.
        let whole: Vec<String> = mods
            .iter()
            .cloned()
            .chain(std::iter::once(name.clone()))
            .collect();
        let expanded = expand_imports(&import_map(&from_file.ast), &whole);
        if expanded != whole {
            // `use crate::a::B;` binds `B` to the literal segments
            // `["crate", "a", "B"]` -- `crate` is a real token in a use
            // tree, not something `expand_imports` strips on its own -- so
            // an expansion is always re-anchored to the crate root exactly
            // as the top-level entry point does for any path.
            let stripped = strip_prefixes(&expanded);
            let (ename, emods) = stripped.split_last()?;
            return Some((emods.to_vec(), ename.clone()));
        }
        if mods.is_empty() {
            // No module qualifier and no import matched: declared in this
            // exact module -- the ordinary case (`impl Bucket` beside
            // `struct Bucket`).
            return Some((from_module.to_vec(), name));
        }
        // A multi-segment bare path with no matching import can only be a
        // child module declared right here (`impl inner::Root`, written in
        // the very file that declares `mod inner;`) -- the one shape Rust
        // allows without `super`/`crate`/a `use`.
        if from_file
            .ast
            .items
            .iter()
            .any(|it| matches!(it, syn::Item::Mod(m) if m.ident == mods[0].as_str()))
        {
            let mut full = from_module.to_vec();
            full.extend(mods.iter().cloned());
            return Some((full, name));
        }
        None
    }

    fn resolve_path(&mut self, raw: &[String]) -> Resolution {
        // `Type::method`, tried once, crate-wide, before any free-function
        // walk: a bare heuristic (the segment right before the last one
        // looks like a type, Rust's own UpperCamelCase convention for one)
        // decides whether to *attempt* this reading at all, but never
        // decides the answer -- `resolve_method_globally` either finds a
        // real declaration both halves agree on, or returns `None` and
        // this falls through to the ordinary free-function path unchanged
        // (so a lowercase module name that happens to precede a free
        // function, `rates::legacy_rate`, never even tries this branch).
        let stripped = strip_prefixes(raw);
        if let Some((method_name, type_segments)) = stripped.split_last()
            && let Some(head) = type_segments.last()
            && starts_uppercase(head)
            && let Some(resolution) = self.resolve_method_globally(type_segments, method_name)
        {
            return resolution;
        }
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
        // `Type::method` claims are resolved once, up front, in
        // `resolve_path` (see `resolve_method_globally`) -- never here.
        // Before 2026-08-27 this frame matched a `Type::method` tail
        // against whatever `impl` blocks happened to sit in *this* file,
        // purely by the type's bare textual name, which is what let
        // `impl super::Root` (written inside a submodule, naming the
        // crate-root's `Root`) satisfy a claim for the submodule's own,
        // unrelated `Root` -- reading one function's contract and calling
        // another's body (adversarial review, "ninth false clean",
        // 2026-08-27). `resolve_method_globally` instead verifies both
        // halves resolve to the *same* declaration before ever looking at
        // an `impl` block's methods.
        let raw = strip_prefixes(segments);
        let segs = expand_imports(&import_map(&file.ast), &raw);
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
                        source_span: source_span(&self.crate_dir, &file.path, f.span()),
                        item: f,
                        canonical: head.clone(),
                        file: file.ast.clone(),
                        unnameable,
                        local: true,
                        is_method: false,
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
                            path: file.path.clone(),
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
        // No nested module named `head`, and `Type::method` is handled
        // entirely up front (see the comment above this function's own
        // `Type::method` note) -- nothing left to try here.
        Resolution::NotFound
    }

    /// `Type::method`, resolved crate-wide by verifying that the type half
    /// and every candidate `impl` block's own `self_ty` name the *same*
    /// declaration, never merely a file or a bare textual name in common.
    ///
    /// This is the fix for the false pass this project's ninth review
    /// found (2026-08-27): before this, a `Type::method` key was matched
    /// against whatever `impl` blocks sat in whichever file the type's own
    /// module segments walked to, purely by `self_ty`'s bare last
    /// identifier -- so `impl super::Root`, written inside `inner.rs` and
    /// plainly naming `crate::Root`, satisfied a claim for
    /// `inner::Root::five` just because the text "Root" matched and the
    /// walk had landed in `inner.rs` for an unrelated reason (module
    /// descent, not type identity). Ply read one function's contract and
    /// called a different function's body.
    ///
    /// The fix has one source of truth: [`Resolver::resolve_type_decl`]
    /// resolves `type_segments` to the module path its `struct`/`enum` is
    /// genuinely *declared* at (following re-exports, same as a free
    /// function's own path does). Every `impl` block in the crate is then
    /// checked by resolving *its own* `self_ty`, from *its own* module
    /// position and file (`resolve_self_ty_target`), and only a match
    /// against that same declared location is accepted as a candidate --
    /// wherever in the crate that `impl` block happens to live. Whatever
    /// canonical path a match earns is therefore read off the *declaration*
    /// the claim and the `impl` block both independently agree on, not
    /// re-spelled from either one alone.
    ///
    /// Returns `None` when nothing in the crate matches at all -- the
    /// caller falls through to `NotFound`, same as an unmatched
    /// free-function name.
    fn resolve_method_globally(
        &mut self,
        type_segments: &[String],
        method_name: &str,
    ) -> Option<Resolution> {
        let modules = self.all_modules();
        let Some((target_path, target_name, _decl_file)) = self.resolve_type_decl(type_segments)
        else {
            // The type half named no `struct`/`enum` this crate declares --
            // it may still be a trait, with `method_name` either a bare
            // signature or a default body. Either way it is a trait
            // method, out of scope the same way a trait-impl override is.
            let (mpath, file) = self.resolve_trait_decl(type_segments)?;
            let _ = mpath;
            let trait_name = type_segments.last()?;
            let t = trait_item_ident(&file.ast.items, trait_name)?;
            for it in &t.items {
                if let syn::TraitItem::Fn(m) = it
                    && m.sig.ident == method_name
                {
                    let has_body = m.default.is_some();
                    return Some(Resolution::Refused(format!(
                        "`{trait_name}::{method_name}` is declared on `trait {trait_name}` ({}), \
                         not in an `impl` block. Ply checks inherent methods and free functions, \
                         not trait methods, yet",
                        if has_body {
                            "a default-body method"
                        } else {
                            "a required method with no body of its own"
                        }
                    )));
                }
            }
            return None;
        };
        let type_text = format_type_path(&target_path, &target_name);
        let mut inherent: Vec<(Vec<String>, bool, SourceFile, syn::ImplItemFn, bool)> = Vec::new();
        let mut trait_impl: Vec<(Vec<String>, bool, SourceFile, syn::ImplItemFn)> = Vec::new();
        for (mpath, file, chain_private) in &modules {
            for item in &file.ast.items {
                let syn::Item::Impl(imp) = item else { continue };
                let Some(this_target) = self.resolve_self_ty_target(mpath, file, &imp.self_ty)
                else {
                    continue;
                };
                if this_target != (target_path.clone(), target_name.clone()) {
                    continue;
                }
                for it in &imp.items {
                    if let syn::ImplItem::Fn(m) = it
                        && m.sig.ident == method_name
                    {
                        if imp.trait_.is_some() {
                            trait_impl.push((
                                mpath.clone(),
                                *chain_private,
                                file.clone(),
                                m.clone(),
                            ));
                        } else {
                            inherent.push((
                                mpath.clone(),
                                *chain_private,
                                file.clone(),
                                m.clone(),
                                !imp.generics.params.is_empty(),
                            ));
                        }
                    }
                }
            }
        }
        // Rust's own resolution rule: an inherent method shadows a trait
        // method of the same name, so `Type::method` means the inherent one
        // whenever both exist. Ambiguity is judged only *within* whichever
        // pool actually applies -- two inherent candidates (crate-wide, not
        // merely within one file: two `impl` blocks for the same type in
        // *different* files are just as ambiguous as two in the same one),
        // or (with no inherent candidate) two trait-impl ones.
        if inherent.len() > 1 {
            return Some(Resolution::Ambiguous(format!(
                "`{type_text}::{method_name}` matches {} different `impl {target_name}` blocks, \
                 each defining its own `{method_name}` -- real Rust when `{target_name}` is \
                 generic and each `impl` targets a different concrete instantiation. Ply's \
                 syntactic reader cannot tell which one a claim means, so it refuses rather than \
                 picking one",
                inherent.len()
            )));
        }
        if let Some((mpath, chain_private, file, m, is_generic)) = inherent.into_iter().next() {
            return Some(self.classify_found_method(
                &mpath,
                chain_private,
                &file,
                &type_text,
                method_name,
                m,
                false,
                is_generic,
            ));
        }
        if trait_impl.len() > 1 {
            return Some(Resolution::Ambiguous(format!(
                "`{type_text}::{method_name}` matches {} different trait `impl`s for \
                 `{target_name}`, each providing its own `{method_name}` -- Ply's syntactic \
                 reader cannot tell which trait a bare `{type_text}::{method_name}` claim means, \
                 so it refuses rather than picking one",
                trait_impl.len()
            )));
        }
        if let Some((mpath, chain_private, file, m)) = trait_impl.into_iter().next() {
            return Some(self.classify_found_method(
                &mpath,
                chain_private,
                &file,
                &type_text,
                method_name,
                m,
                true,
                false,
            ));
        }
        // The type is real and its declaration is genuinely known, but
        // nothing implementing it defines this method name.
        None
    }

    /// One matched `impl`-block method, sorted into `Found` (a plain
    /// inherent, non-generic, receiverless associated function -- the same
    /// shape a free function already is, from here on) or `Refused` (a
    /// trait method, a generic `impl` block, or a receiver this task does
    /// not build) — in that priority order, since a trait-impl method is
    /// out of scope regardless of whether its `impl` block also happens to
    /// be generic or the method also happens to take a receiver.
    ///
    /// `mpath`/`chain_private` are the *`impl` block's own* module position
    /// -- which may differ from `target_path` (the type's own declaration
    /// site) when the `impl` lives in a different file from its type, a
    /// shape real Rust allows and this resolver now supports. Nameability
    /// from the crate-root harness is a fact about where the *method text*
    /// sits, so it is judged against the `impl`'s own location, never the
    /// type's.
    #[allow(clippy::too_many_arguments)]
    fn classify_found_method(
        &self,
        mpath: &[String],
        chain_private: bool,
        file: &SourceFile,
        type_text: &str,
        method_name: &str,
        m: syn::ImplItemFn,
        is_trait_impl: bool,
        impl_is_generic: bool,
    ) -> Resolution {
        if is_trait_impl {
            return Resolution::Refused(format!(
                "`{type_text}::{method_name}` is defined in a trait implementation (`impl ... for \
                 {type_text}`). Ply checks inherent methods and free functions, not trait \
                 methods, yet"
            ));
        }
        if impl_is_generic {
            return Resolution::Refused(format!(
                "`{type_text}::{method_name}` is declared in a generic `impl` block (`impl<...> \
                 {type_text}<...>`). Ply does not check generic `impl` blocks yet"
            ));
        }
        if has_self_receiver(&m.sig) {
            return Resolution::Refused(receiver_refusal_reason(type_text, method_name, &m));
        }
        let unnameable = (chain_private
            || (!mpath.is_empty() && matches!(m.vis, syn::Visibility::Inherited)))
        .then(|| {
            format!(
                "`{type_text}::{method_name}` is private to the module it is declared in, so the \
                 harness Ply generates at the crate root cannot call it by name"
            )
        });
        Resolution::Found(Box::new(FoundFn {
            source_span: source_span(&self.crate_dir, &file.path, m.span()),
            item: impl_fn_to_item_fn(&m),
            canonical: format!("{type_text}::{method_name}"),
            file: file.ast.clone(),
            unnameable,
            local: true,
            is_method: true,
        }))
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
                // `Type::method` no longer resolves through
                // `resolve_in_file` at all (see `resolve_method_globally`),
                // so this branch is unreachable in practice; kept matching
                // `Resolution`'s full set rather than a wildcard, so a
                // future variant cannot silently fall through here
                // unnoticed.
                Resolution::Refused(_) | Resolution::Ambiguous(_) => {}
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
                path: path.to_path_buf(),
            });
        self.files.insert(path.to_path_buf(), parsed.clone());
        parsed
    }
}

pub(crate) fn source_span(
    crate_dir: &Path,
    path: &Path,
    span: proc_macro2::Span,
) -> crate::diag::Span {
    let relative = path
        .strip_prefix(crate_dir)
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let base: Vec<_> = crate_dir.components().collect();
            let target: Vec<_> = path.components().collect();
            let common = base
                .iter()
                .zip(&target)
                .take_while(|(left, right)| left == right)
                .count();
            let mut out = PathBuf::new();
            for _ in common..base.len() {
                out.push("..");
            }
            for component in &target[common..] {
                out.push(component.as_os_str());
            }
            out
        });
    let start = span.start();
    let end = span.end();
    let start = [start.line.saturating_sub(1) as u32, start.column as u32];
    let mut end = [end.line.saturating_sub(1) as u32, end.column as u32];
    if end < start {
        end = start;
    }
    crate::diag::Span {
        file: relative.to_string_lossy().replace('\\', "/"),
        start,
        end,
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

/// Whether `name` is declared as a `struct` or `enum` right in `items` --
/// the ground truth a `Type::method` claim's type half must resolve to
/// (never an `impl` block, and never a re-export, both of which merely
/// *point at* a declaration that lives elsewhere).
fn type_item_ident(items: &[syn::Item], name: &str) -> Option<String> {
    items.iter().find_map(|item| match item {
        syn::Item::Struct(s) if s.ident == name => Some(s.ident.to_string()),
        syn::Item::Enum(e) if e.ident == name => Some(e.ident.to_string()),
        _ => None,
    })
}

/// The `trait` item named `name`, declared right in `items` -- the
/// declaration side of the trait-method fallback in
/// `Resolver::resolve_method_globally`.
fn trait_item_ident<'a>(items: &'a [syn::Item], name: &str) -> Option<&'a syn::ItemTrait> {
    items.iter().find_map(|item| match item {
        syn::Item::Trait(t) if t.ident == name => Some(t),
        _ => None,
    })
}

/// A type's declared module path plus its own name, spelled the way a
/// `Type::method` claim (or an `unnameable`/canonical string) writes it:
/// bare at the crate root, module-qualified otherwise.
fn format_type_path(module_path: &[String], name: &str) -> String {
    if module_path.is_empty() {
        name.to_string()
    } else {
        format!("{}::{name}", module_path.join("::"))
    }
}

/// A very small, deliberately conservative textual check used only to
/// *name* a second blocking reason alongside a receiver refusal (never to
/// gate a check that has already resolved normally -- that remains
/// `harness::RustType`'s job, the one place this project keeps that
/// authority). Flags a bare identifier that is neither one of Rust's
/// built-in scalar names nor `Self` -- almost always a custom
/// `struct`/`enum` this resolver has no way to build a value of, and the
/// common shape a receiver-refused method's *other* parameters take.
/// Deliberately narrow: a false negative here only means a second reason
/// goes unnamed (the receiver reason, always named, remains true), never a
/// false claim that a genuinely-supported type is not.
fn obviously_unsupported_param(ty: &syn::Type) -> Option<String> {
    const KNOWN: &[&str] = &[
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        "bool", "f32", "f64", "Self",
    ];
    let syn::Type::Path(p) = ty else {
        return None;
    };
    let seg = p.path.segments.last()?;
    let name = seg.ident.to_string();
    if KNOWN.contains(&name.as_str()) || name.starts_with("NonZero") || name == "Duration" {
        return None;
    }
    Some(name)
}

/// A receiver refusal names *every* reason the method cannot be checked
/// yet, not only the first one found -- the review's own finding
/// (2026-08-27): a `&mut self` method is missing two things, not one
/// (building the receiver, and a way to state what the method *changes*),
/// and a receiver alongside a parameter type this resolver cannot build
/// either would otherwise read as "fix the receiver and you're done", which
/// is false.
fn receiver_refusal_reason(type_text: &str, method_name: &str, m: &syn::ImplItemFn) -> String {
    let mut reasons = vec![format!(
        "cannot yet build a value of `{type_text}` to call it on -- constructing a receiver is \
         not supported yet"
    )];
    if let Some(syn::FnArg::Receiver(r)) = m.sig.inputs.first()
        && r.mutability.is_some()
    {
        reasons.push(
            "even a built receiver would not be enough here: this method takes `&mut self`, and \
             Ply has no way yet to state what it is supposed to change about the value it was \
             called on, so there would still be nothing to check"
                .to_string(),
        );
    }
    for arg in m.sig.inputs.iter().skip(1) {
        if let syn::FnArg::Typed(pt) = arg
            && let Some(bad_ty) = obviously_unsupported_param(&pt.ty)
        {
            reasons.push(format!(
                "and separately, its parameter of type `{bad_ty}` is a shape Ply's checkers do \
                 not build inputs for either, so building a receiver would not be enough on its \
                 own"
            ));
        }
    }
    format!(
        "Ply found `{type_text}::{method_name}` but {}",
        reasons.join("; ")
    )
}

/// Whether this signature's first argument is a receiver (`self`, `&self`,
/// `&mut self`) — the one shape this task's scope defers, because building
/// a value to call it on is unsettled (docs/review-self-construction.md).
fn has_self_receiver(sig: &syn::Signature) -> bool {
    matches!(sig.inputs.first(), Some(syn::FnArg::Receiver(_)))
}

/// An `impl`-block method's fields are the same shape a free function's
/// are (`attrs`, `vis`, `sig`, `block`) — `syn::ImplItemFn` just names them
/// for a different context. Rebuilding a plain `syn::ItemFn` out of one lets
/// every downstream reader (`build_contract_fn`, the call-site collector)
/// stay written against free functions and never need its own method-shaped
/// twin.
fn impl_fn_to_item_fn(m: &syn::ImplItemFn) -> syn::ItemFn {
    syn::ItemFn {
        attrs: m.attrs.clone(),
        vis: m.vis.clone(),
        sig: m.sig.clone(),
        block: Box::new(m.block.clone()),
    }
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

/// Raw signature text (params keep any `&`, unlike `harness::ContractFn`'s
/// normalised `Param`), for a stub whose signature must match the callee's
/// own (Kani checks a stub's signature against its target). `pub` so D5's
/// first branch (§5.5) can build a same-crate `Assumed` fallback the same
/// way `Assumed`'s own `CalleeStatus` variant does.
pub fn signature_of(f: &syn::ItemFn) -> CalleeSignature {
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
    fn resolved_functions_keep_exact_workspace_relative_item_ranges() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/rates.rs"),
            "// file module\npub fn quote(x: u32) -> u32 {\n    x + 1\n}\n",
        )
        .unwrap();
        let src = "pub fn root() -> u32 {\n    1\n}\n\npub mod inline {\n    pub fn quote() -> u32 { 2 }\n}\npub mod rates;\n";
        let mut resolver = Resolver::new(src, dir.path(), BTreeMap::new()).unwrap();

        let root = match resolver.lookup_fn("root") {
            Resolution::Found(found) => found.source_span,
            other => panic!("root should resolve, got {}", describe(&other)),
        };
        assert_eq!(root.file, "src/lib.rs");
        assert_eq!(root.start, [0, 0]);
        assert_eq!(root.end, [2, 1]);

        let inline = match resolver.lookup_fn("inline::quote") {
            Resolution::Found(found) => found.source_span,
            other => panic!("inline fn should resolve, got {}", describe(&other)),
        };
        assert_eq!(inline.file, "src/lib.rs");
        assert_eq!(inline.start, [5, 4]);
        assert_eq!(inline.end, [5, 31]);

        let file_module = match resolver.lookup_fn("rates::quote") {
            Resolution::Found(found) => found.source_span,
            other => panic!("file-module fn should resolve, got {}", describe(&other)),
        };
        assert_eq!(file_module.file, "src/rates.rs");
        assert_eq!(file_module.start, [1, 0]);
        assert_eq!(file_module.end, [3, 1]);
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

    // -- the ninth false clean (adversarial review, 2026-08-27): a
    // `Type::method` claim's type half must resolve to the same
    // *declaration* an `impl` block's own `self_ty` names, never merely a
    // bare textual match against whatever file the module-path walk landed
    // on. Every test below needs more than one module -- the one shape the
    // suite that shipped this defect never exercised (§9: a defect found by
    // review enters the suite as a fixture of its own shape).

    /// A short, readable stand-in for `{:?}` -- `Resolution` cannot derive
    /// `Debug` (its `FoundFn` payload carries `syn` types this workspace
    /// does not build with the `extra-traits` feature), so a test failure
    /// needs this to say which variant it got.
    fn describe(r: &Resolution) -> String {
        match r {
            Resolution::Found(f) => format!("Found({})", f.canonical),
            Resolution::Opaque(s) => format!("Opaque({s})"),
            Resolution::NotFound => "NotFound".to_string(),
            Resolution::Refused(s) => format!("Refused({s})"),
            Resolution::Ambiguous(s) => format!("Ambiguous({s})"),
        }
    }

    /// The exact reproduction: two structs both named `Root`, one at the
    /// crate root and one in `inner`. The `impl` block written inside
    /// `inner.rs` says `impl super::Root` -- the CRATE ROOT's `Root`, not
    /// `inner`'s own -- and carries a promise that is false of its own
    /// body. The old resolver matched `inner::Root::five` against this
    /// block purely because the bare name "Root" and the recursion frame
    /// lined up; that promise must never again attach to a function it
    /// does not describe.
    fn write_wrongfn_crate(dir: &Path) -> String {
        std::fs::create_dir_all(dir.join("src/inner")).unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "pub mod inner;\n\
             pub struct Root;\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/inner.rs"),
            "pub mod sub;\n\
             pub struct Root;\n\
             impl super::Root {\n\
             #[ply::ensures(|result| *result == 999)]\n\
             pub fn five() -> u32 { 5 }\n\
             }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/inner/sub.rs"),
            "impl super::Root {\n\
             pub fn five() -> u32 { 999 }\n\
             }\n",
        )
        .unwrap();
        std::fs::read_to_string(dir.join("src/lib.rs")).unwrap()
    }

    #[test]
    fn the_wrong_spelling_no_longer_attaches_a_false_promise_to_the_wrong_function() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_wrongfn_crate(dir.path());
        let mut r = Resolver::new(&src, dir.path(), BTreeMap::new()).unwrap();
        match r.lookup_fn("inner::Root::five") {
            Resolution::Found(found) => {
                // The only honest match for THIS spelling is `inner::Root`'s
                // own `five` (in `sub.rs`), which carries no promise at
                // all -- never the crate-root `Root::five` whose promise
                // ("the answer is 999") is false of a body that returns 5.
                assert!(
                    !has_inline_contract(&found.item),
                    "`inner::Root::five` must resolve to the function actually declared as \
                     `inner::Root`'s own -- which carries no promise -- never to the unrelated \
                     crate-root `Root` whose `impl` block merely happens to sit in the same file"
                );
                assert_eq!(found.canonical, "inner::Root::five");
            }
            other => {
                // A refusal is also an honest outcome -- anything except a
                // clean match to the wrong body.
                panic!(
                    "expected a real resolution to inner::Root's own five, got {}",
                    describe(&other)
                );
            }
        }
    }

    #[test]
    fn the_correct_spelling_resolves_to_the_function_that_actually_carries_the_promise() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_wrongfn_crate(dir.path());
        let mut r = Resolver::new(&src, dir.path(), BTreeMap::new()).unwrap();
        match r.lookup_fn("Root::five") {
            Resolution::Found(found) => {
                assert_eq!(
                    found.canonical, "Root::five",
                    "the promise lives on the crate-root `Root`, so that is what `Root::five` \
                     must canonicalise to"
                );
                assert!(
                    has_inline_contract(&found.item),
                    "`Root::five` must resolve to the function that actually carries the \
                     promise (\"the answer is 999\"), not to `inner::Root`'s unrelated `five`"
                );
            }
            other => panic!(
                "`Root::five` names a real function and must resolve: {}",
                describe(&other)
            ),
        }
    }

    /// A correct claim in a multi-module crate still resolves and checks --
    /// the ordinary, non-adversarial case the fix must not break.
    #[test]
    fn a_correct_claim_in_a_multi_module_crate_still_resolves() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/inner")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub mod inner;\n").unwrap();
        std::fs::write(
            dir.path().join("src/inner.rs"),
            "pub mod sub;\n\
             pub struct Widget;\n\
             impl Widget {\n\
             #[ply::ensures(|result| *result == 3)]\n\
             pub fn value() -> u32 { 3 }\n\
             }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/inner/sub.rs"), "// unrelated\n").unwrap();
        let src = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        let mut r = Resolver::new(&src, dir.path(), BTreeMap::new()).unwrap();
        match r.lookup_fn("inner::Widget::value") {
            Resolution::Found(found) => {
                assert_eq!(found.canonical, "inner::Widget::value");
                assert!(has_inline_contract(&found.item));
            }
            other => panic!("expected a clean resolution, got {}", describe(&other)),
        }
    }

    /// A type re-exported under another name: the `impl` block is written
    /// against the type's real declaration, and a claim spelled with the
    /// alias must still land on it -- re-exports are followed for a type
    /// exactly as they already are for a free function.
    #[test]
    fn a_type_re_exported_under_another_name_still_resolves_its_methods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub mod inner;\n\
             pub use inner::Root as Exported;\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/inner.rs"),
            "pub struct Root;\n\
             impl Root {\n\
             #[ply::ensures(|result| *result == 5)]\n\
             pub fn five() -> u32 { 5 }\n\
             }\n",
        )
        .unwrap();
        let src = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        let mut r = Resolver::new(&src, dir.path(), BTreeMap::new()).unwrap();
        match r.lookup_fn("Exported::five") {
            Resolution::Found(found) => {
                assert_eq!(
                    found.canonical, "inner::Root::five",
                    "the canonical path is the type's real declaration, not the alias a claim \
                     happened to spell it with"
                );
                assert!(has_inline_contract(&found.item));
            }
            other => panic!(
                "a re-exported name must still resolve: {}",
                describe(&other)
            ),
        }
    }

    /// An `impl` block in a different file from its type's own declaration
    /// -- real, ordinary Rust (a `struct` in one module, its methods added
    /// from another) -- must still resolve.
    #[test]
    fn an_impl_block_in_a_different_file_from_its_type_still_resolves() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub mod types;\n\
             pub mod ops;\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/types.rs"), "pub struct Widget;\n").unwrap();
        std::fs::write(
            dir.path().join("src/ops.rs"),
            "use crate::types::Widget;\n\
             impl Widget {\n\
             #[ply::ensures(|result| *result == 9)]\n\
             pub fn nine() -> u32 { 9 }\n\
             }\n",
        )
        .unwrap();
        let src = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        let mut r = Resolver::new(&src, dir.path(), BTreeMap::new()).unwrap();
        match r.lookup_fn("types::Widget::nine") {
            Resolution::Found(found) => {
                assert_eq!(found.canonical, "types::Widget::nine");
                assert!(has_inline_contract(&found.item));
            }
            other => panic!(
                "a type and the `impl` block implementing it may live in different files -- \
                 real Rust -- and must still resolve: {}",
                describe(&other)
            ),
        }
    }

    /// Ambiguity is not scoped to one file: two `impl` blocks for the same
    /// type in *different* files are exactly as ambiguous as two in the
    /// same file, and Ply must refuse rather than silently pick the first
    /// one it walks to.
    #[test]
    fn two_impl_blocks_for_one_type_in_different_files_are_ambiguous_not_silently_picked() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub mod types;\n\
             pub mod a;\n\
             pub mod b;\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/types.rs"), "pub struct Widget;\n").unwrap();
        std::fs::write(
            dir.path().join("src/a.rs"),
            "use crate::types::Widget;\n\
             impl Widget { pub fn describe() -> u32 { 1 } }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/b.rs"),
            "use crate::types::Widget;\n\
             impl Widget { pub fn describe() -> u32 { 2 } }\n",
        )
        .unwrap();
        let src = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        let mut r = Resolver::new(&src, dir.path(), BTreeMap::new()).unwrap();
        match r.lookup_fn("types::Widget::describe") {
            Resolution::Ambiguous(reason) => {
                assert!(
                    reason.contains("Widget") && reason.contains("describe"),
                    "the refusal must name the type and method: {reason}"
                );
            }
            other => panic!(
                "two impl blocks for one type, in different files, defining the same method \
                 name must be refused as ambiguous, not silently resolved to whichever file \
                 the walk reached first: {}",
                describe(&other)
            ),
        }
    }

    // -- a receiver refusal names every blocking reason, not only the
    // first (adversarial review, 2026-08-27): a `&mut self` method is
    // missing two things, and a receiver plus an unbuildable argument
    // names only the receiver was true before this.

    #[test]
    fn a_mut_self_method_names_both_the_receiver_and_the_mutation_gap() {
        let src = r#"
pub struct Bucket { n: u32 }
impl Bucket {
    pub fn bump(&mut self) { self.n += 1; }
}
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        match r.lookup_fn("Bucket::bump") {
            Resolution::Refused(reason) => {
                assert!(
                    reason.contains("receiver"),
                    "must still name the receiver blocker: {reason}"
                );
                assert!(
                    reason.contains("&mut self") || reason.contains("change"),
                    "must ALSO name that Ply has no way to state what a `&mut self` method \
                     changes -- a second, real blocker a fixed receiver would not remove: \
                     {reason}"
                );
            }
            other => panic!("expected Refused, got {}", describe(&other)),
        }
    }

    #[test]
    fn a_receiver_plus_an_unbuildable_argument_names_both_reasons() {
        let src = r#"
pub struct Thing;
pub struct Odd;
impl Thing {
    pub fn scale(&self, factor: Odd) -> u32 { 0 }
}
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        match r.lookup_fn("Thing::scale") {
            Resolution::Refused(reason) => {
                assert!(
                    reason.contains("receiver"),
                    "must name the receiver blocker: {reason}"
                );
                assert!(
                    reason.contains("Odd"),
                    "must ALSO name the parameter type Ply cannot build inputs for -- fixing the \
                     receiver alone would not be enough: {reason}"
                );
            }
            other => panic!("expected Refused, got {}", describe(&other)),
        }
    }

    #[test]
    fn a_plain_receiver_refusal_names_only_the_receiver_when_that_is_the_only_reason() {
        let src = r#"
pub struct Bucket { n: u32 }
impl Bucket {
    pub fn n(&self) -> u32 { self.n }
}
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        match r.lookup_fn("Bucket::n") {
            Resolution::Refused(reason) => {
                assert!(reason.contains("receiver"), "{reason}");
                assert!(
                    !reason.contains("&mut self") && !reason.contains("change"),
                    "a `&self` method has no mutation gap to name: {reason}"
                );
            }
            other => panic!("expected Refused, got {}", describe(&other)),
        }
    }

    // -- the "a `Self` answer is always fine" rule's own blind spot on the
    // sampling tier (adversarial review, 2026-08-27): `private_field_names`
    // is what a caller checks before trusting that rule where it does not
    // hold.

    #[test]
    fn private_field_names_lists_only_the_non_pub_fields() {
        let src = r#"
pub struct Bucket {
    pub capacity: u32,
    filled: u32,
    pub(crate) note: u32,
}
"#;
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        let mut fields = r.private_field_names("Bucket").unwrap();
        fields.sort();
        assert_eq!(
            fields,
            vec!["filled".to_string(), "note".to_string()],
            "a fully `pub` field must not be listed; a private or `pub(crate)` one must be"
        );
    }

    #[test]
    fn private_field_names_is_none_for_a_type_that_is_not_a_struct() {
        let src = "pub enum Shape { Round, Square }\n";
        let mut r = Resolver::new(src, Path::new("."), BTreeMap::new()).unwrap();
        assert_eq!(r.private_field_names("Shape"), None);
        assert_eq!(r.private_field_names("Nowhere"), None);
    }
}
