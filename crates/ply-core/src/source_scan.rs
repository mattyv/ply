//! Reading a crate's real source into the observed model the conformance
//! comparison judges.
//!
//! This is the half that looks. It walks one package's modules from the
//! crate root, records what each item refers to, and -- the part that
//! matters most -- records what it could not read, so the comparison can
//! tell "no forbidden reference here" apart from "this scan did not see
//! that code".
//!
//! **The supported slice is deliberately narrow, and named.** It resolves
//! paths written `crate::`, `self::`, `super::`, a module in scope, and
//! names brought in by a plain `use`. Everything else -- a glob import, a
//! path built by a macro, a `#[path]` module, a trait or generic call --
//! is recorded as a gap or an unresolved destination rather than guessed
//! at. Widening the slice is a later increment; quietly widening it by
//! guessing would make the answer worse, not better.
//!
//! Calls that leave the crate are not this tier's business: it compares
//! modules inside one package, and a package's dependency on another
//! package is the crate tier's statement to make.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::source_model::{
    BuildContext, CoverageGap, Destination, GapScope, ItemId, ItemKind, ItemRecord, ModuleId,
    Reference, ReferenceKind, SourceModel, Span, Visibility,
};

/// Scan one package's library source, starting from its crate root file.
pub fn scan_crate(root_file: &Path, krate: &str, repo_root: &Path) -> SourceModel {
    let mut model = SourceModel::new(BuildContext::simple(krate, "lib"));
    let mut scan = Scan {
        krate: krate.to_string(),
        repo_root: repo_root.to_path_buf(),
        model: &mut model,
        method_calls_here: 0,
    };
    scan.module_file(root_file, ModuleId::root(krate));
    // Second pass: resolve every recorded path against the module tree now
    // that the whole tree is known. Resolving as we went would make a
    // forward reference look unresolvable.
    resolve(&mut model);
    model
}

struct Scan<'a> {
    krate: String,
    repo_root: PathBuf,
    model: &'a mut SourceModel,
    /// Method calls seen in the module being read. Counted rather than
    /// listed -- see the note on the visitor field of the same name.
    method_calls_here: usize,
}

/// A path a reference used, kept as written until the whole tree is known.
/// Held in `Destination::Unresolved` until [`resolve`] replaces it.
const PENDING: &str = "ply-pending:";

impl Scan<'_> {
    fn rel(&self, file: &Path) -> String {
        file.strip_prefix(&self.repo_root)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string()
    }

    fn module_file(&mut self, file: &Path, module: ModuleId) {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                self.model.gaps.push(CoverageGap {
                    scope: GapScope::Module(module.clone()),
                    cause: format!("its source file could not be read ({e})"),
                    affects: all_kinds(),
                });
                self.model.modules.push(module);
                return;
            }
        };
        let parsed = match syn::parse_file(&text) {
            Ok(f) => f,
            Err(e) => {
                self.model.gaps.push(CoverageGap {
                    scope: GapScope::Module(module.clone()),
                    cause: format!("its source could not be parsed ({e})"),
                    affects: all_kinds(),
                });
                self.model.modules.push(module);
                return;
            }
        };
        self.model.modules.push(module.clone());
        let imports = collect_imports(&parsed.items, &self.krate);
        let outer = std::mem::take(&mut self.method_calls_here);
        self.items(&parsed.items, &module, file, &imports);
        self.note_method_calls(&module);
        self.method_calls_here = outer;
    }

    /// One line per module for the method calls this scan could not follow.
    fn note_method_calls(&mut self, module: &ModuleId) {
        let n = std::mem::take(&mut self.method_calls_here);
        if n == 0 {
            return;
        }
        self.model.gaps.push(CoverageGap {
            scope: GapScope::Module(module.clone()),
            cause: format!(
                "it makes {n} method call{} whose target this scan does not work out -- a \
                 method call needs the receiver's type, which this scan does not compute",
                if n == 1 { "" } else { "s" }
            ),
            affects: vec![ReferenceKind::Call],
        });
    }

    fn items(
        &mut self,
        items: &[syn::Item],
        module: &ModuleId,
        file: &Path,
        imports: &BTreeMap<String, String>,
    ) {
        for item in items {
            match item {
                syn::Item::Mod(m) => self.module_item(m, module, file),
                syn::Item::Fn(f) => {
                    let id = ItemId::new(module.clone(), &f.sig.ident.to_string());
                    self.model.items.push(ItemRecord {
                        id: id.clone(),
                        kind: ItemKind::Function,
                        visibility: visibility(&f.vis),
                        span: self.span(file, &f.sig.ident),
                    });
                    self.body(&f.block, &id, file, imports);
                }
                syn::Item::Impl(imp) => {
                    for it in &imp.items {
                        if let syn::ImplItem::Fn(m) = it {
                            let id = ItemId::new(module.clone(), &m.sig.ident.to_string());
                            self.model.items.push(ItemRecord {
                                id: id.clone(),
                                kind: ItemKind::Function,
                                visibility: visibility(&m.vis),
                                span: self.span(file, &m.sig.ident),
                            });
                            self.body(&m.block, &id, file, imports);
                        }
                    }
                }
                // Opaque: a macro can expand to anything, including a call
                // across the very boundary a rule is about.
                syn::Item::Macro(m) => self.model.gaps.push(CoverageGap {
                    scope: GapScope::Module(module.clone()),
                    cause: format!(
                        "`{}!` expands to code this scan cannot see",
                        m.mac
                            .path
                            .segments
                            .last()
                            .map(|s| s.ident.to_string())
                            .unwrap_or_default()
                    ),
                    affects: all_kinds(),
                }),
                _ => {}
            }
        }
    }

    fn module_item(&mut self, m: &syn::ItemMod, parent: &ModuleId, file: &Path) {
        // A `#[path]` module is read where it says, or not at all -- never
        // by trying the conventional filename instead.
        let has_path_attr = m.attrs.iter().any(|a| a.path().is_ident("path"));
        let mut child = parent.clone();
        child.path.push(m.ident.to_string());

        if let Some((_, inner)) = &m.content {
            self.model.modules.push(child.clone());
            let imports = collect_imports(inner, &self.krate);
            let outer = std::mem::take(&mut self.method_calls_here);
            self.items(inner, &child, file, &imports);
            self.note_method_calls(&child);
            self.method_calls_here = outer;
            return;
        }
        if has_path_attr {
            self.model.gaps.push(CoverageGap {
                scope: GapScope::Module(child.clone()),
                cause: "it is declared with a `#[path]` attribute, which this scan does not \
                        follow, so its contents were not read"
                    .into(),
                affects: all_kinds(),
            });
            self.model.modules.push(child);
            return;
        }
        // `foo.rs` beside the parent, or `foo/mod.rs` under it.
        let dir = file.parent().unwrap_or(Path::new("."));
        let name = m.ident.to_string();
        let flat = dir.join(format!("{name}.rs"));
        let nested = dir.join(&name).join("mod.rs");
        let sub = dir.join(&name).join(format!("{name}.rs"));
        let target = [flat, nested, sub].into_iter().find(|p| p.exists());
        match target {
            Some(t) => self.module_file(&t, child),
            None => {
                self.model.gaps.push(CoverageGap {
                    scope: GapScope::Module(child.clone()),
                    cause: "this scan could not find the file declaring it".into(),
                    affects: all_kinds(),
                });
                self.model.modules.push(child);
            }
        }
    }

    /// Every path a function body mentions, recorded as written.
    fn body(
        &mut self,
        block: &syn::Block,
        origin: &ItemId,
        file: &Path,
        imports: &BTreeMap<String, String>,
    ) {
        use syn::visit::Visit;
        struct V<'a> {
            origin: &'a ItemId,
            imports: &'a BTreeMap<String, String>,
            krate: &'a str,
            file: String,
            out: Vec<Reference>,
            gaps: Vec<CoverageGap>,
            /// Counted, not listed. A method call's destination needs the
            /// receiver's type, which this scan does not compute -- but a
            /// finding for every `.len()` and `.trim()` would bury the
            /// report, and a report nobody reads protects nobody. One line
            /// per module keeps the limit visible and the page usable.
            method_calls: usize,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_expr_call(&mut self, e: &'ast syn::ExprCall) {
                if let syn::Expr::Path(p) = &*e.func {
                    let segs: Vec<String> = p
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect();
                    if let Some(path) = first_party(&segs, self.imports, self.krate, self.origin) {
                        self.out.push(Reference {
                            origin: self.origin.clone(),
                            destination: Destination::Unresolved {
                                reason: format!("{PENDING}{path}"),
                            },
                            kind: ReferenceKind::Call,
                            span: Span::at(&self.file, line_of(&p.path)),
                        });
                    }
                }
                syn::visit::visit_expr_call(self, e);
            }
            /// A method call's destination needs the receiver's type, which
            /// this scan does not compute. Recorded as a gap at the site,
            /// so a rule it might have broken is not called checked.
            fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
                self.method_calls += 1;
                syn::visit::visit_expr_method_call(self, e);
            }
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                let name = m
                    .path
                    .segments
                    .last()
                    .map(|s| s.ident.to_string())
                    .unwrap_or_default();
                // The common formatting macros take values, not paths, and
                // treating every one as an opaque hole would bury the
                // report in noise for no gain.
                if !matches!(
                    name.as_str(),
                    "format"
                        | "println"
                        | "print"
                        | "eprintln"
                        | "eprint"
                        | "write"
                        | "writeln"
                        | "vec"
                        | "assert"
                        | "assert_eq"
                        | "assert_ne"
                        | "panic"
                        | "todo"
                        | "unimplemented"
                        | "unreachable"
                        | "dbg"
                ) {
                    self.gaps.push(CoverageGap {
                        scope: GapScope::Site {
                            origin: self.origin.clone(),
                            span: Span::at(&self.file, 0),
                        },
                        cause: format!("`{name}!` expands to code this scan cannot see"),
                        affects: all_kinds(),
                    });
                }
                syn::visit::visit_macro(self, m);
            }
        }
        let mut v = V {
            origin,
            imports,
            krate: &self.krate,
            file: self.rel(file),
            out: Vec::new(),
            gaps: Vec::new(),
            method_calls: 0,
        };
        v.visit_block(block);
        let method_calls = v.method_calls;
        self.model.references.extend(v.out);
        self.model.gaps.extend(v.gaps);
        self.method_calls_here += method_calls;
    }

    fn span(&self, file: &Path, ident: &syn::Ident) -> Span {
        Span::at(&self.rel(file), ident.span().start().line as u32)
    }
}

fn line_of(path: &syn::Path) -> u32 {
    path.segments
        .first()
        .map(|s| s.ident.span().start().line as u32)
        .unwrap_or(0)
}

fn all_kinds() -> Vec<ReferenceKind> {
    vec![
        ReferenceKind::Call,
        ReferenceKind::FunctionValue,
        ReferenceKind::Type,
        ReferenceKind::Import,
    ]
}

fn visibility(v: &syn::Visibility) -> Visibility {
    match v {
        syn::Visibility::Public(_) => Visibility::Public,
        syn::Visibility::Restricted(_) => Visibility::Crate,
        syn::Visibility::Inherited => Visibility::Private,
    }
}

/// Names a `use` brings into this file, mapped to the full path they mean.
/// Only first-party paths matter here.
fn collect_imports(items: &[syn::Item], krate: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for item in items {
        if let syn::Item::Use(u) = item {
            walk_use(&u.tree, String::new(), krate, &mut out);
        }
    }
    out
}

fn walk_use(tree: &syn::UseTree, prefix: String, krate: &str, out: &mut BTreeMap<String, String>) {
    let join = |p: &str, s: &str| {
        if p.is_empty() {
            s.to_string()
        } else {
            format!("{p}::{s}")
        }
    };
    match tree {
        syn::UseTree::Path(p) => {
            let name = p.ident.to_string();
            walk_use(&p.tree, join(&prefix, &name), krate, out);
        }
        syn::UseTree::Name(n) => {
            let name = n.ident.to_string();
            let full = join(&prefix, &name);
            if is_first_party_root(&full, krate) {
                out.insert(name, full);
            }
        }
        syn::UseTree::Rename(r) => {
            let full = join(&prefix, &r.ident.to_string());
            if is_first_party_root(&full, krate) {
                out.insert(r.rename.to_string(), full);
            }
        }
        // A glob brings in names this scan cannot enumerate, so a bare name
        // later on may come from here. Left unrecorded on purpose: guessing
        // is what makes a wrong answer look like a right one.
        syn::UseTree::Glob(_) => {}
        syn::UseTree::Group(g) => {
            for t in &g.items {
                walk_use(t, prefix.clone(), krate, out);
            }
        }
    }
}

fn is_first_party_root(path: &str, krate: &str) -> bool {
    let first = path.split("::").next().unwrap_or_default();
    first == "crate" || first == "self" || first == "super" || first == krate
}

/// Turn a path as written into a crate-absolute path, or `None` when it
/// leaves this crate (another package's function is the crate tier's
/// business, not this one's).
fn first_party(
    segs: &[String],
    imports: &BTreeMap<String, String>,
    krate: &str,
    origin: &ItemId,
) -> Option<String> {
    if segs.is_empty() {
        return None;
    }
    let head = &segs[0];
    let rest = || segs[1..].join("::");

    if head == "crate" || head == krate {
        return Some(normalise(&format!("{krate}::{}", rest()), krate, origin));
    }
    if head == "self" {
        return Some(normalise(
            &format!("{}::{}", origin.module, rest()),
            krate,
            origin,
        ));
    }
    if head == "super" {
        let mut m = origin.module.clone();
        m.path.pop();
        return Some(normalise(&format!("{m}::{}", rest()), krate, origin));
    }
    if let Some(full) = imports.get(head) {
        let tail = rest();
        let joined = if tail.is_empty() {
            full.clone()
        } else {
            format!("{full}::{tail}")
        };
        return Some(normalise(&joined, krate, origin));
    }
    // A bare name with no import: a function in this very module, if it is
    // a single segment. More than one segment means a module path this
    // scan cannot place.
    if segs.len() == 1 {
        return Some(format!("{}::{}", origin.module, head));
    }
    None
}

/// Rewrite `crate`/`self`/`super` heads into the crate's own name.
fn normalise(path: &str, krate: &str, origin: &ItemId) -> String {
    let mut parts: Vec<String> = path.split("::").map(|s| s.to_string()).collect();
    if parts.first().map(String::as_str) == Some("crate") {
        parts[0] = krate.to_string();
    }
    if parts.first().map(String::as_str) == Some("self") {
        let mut m: Vec<String> = origin
            .module
            .to_string()
            .split("::")
            .map(String::from)
            .collect();
        m.extend(parts.into_iter().skip(1));
        parts = m;
    }
    parts.join("::")
}

/// Second pass: turn each recorded path into a definite item where the
/// module tree really has it, and leave an honest reason where it does not.
fn resolve(model: &mut SourceModel) {
    let known: Vec<String> = model.modules.iter().map(|m| m.to_string()).collect();
    for r in &mut model.references {
        let Destination::Unresolved { reason } = &r.destination else {
            continue;
        };
        let Some(path) = reason.strip_prefix(PENDING) else {
            continue;
        };
        let path = path.to_string();
        let Some((module_str, name)) = path.rsplit_once("::") else {
            r.destination = Destination::Unresolved {
                reason: format!("`{path}` is not a path this scan can place"),
            };
            continue;
        };
        if known.iter().any(|m| m == module_str) {
            r.destination = Destination::Definite(ItemId::new(ModuleId::parse(module_str), name));
        } else {
            r.destination = Destination::Unresolved {
                reason: format!("`{path}` does not name a module this scan found in this crate"),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/modtier")
    }

    fn scan_fixture() -> SourceModel {
        let root = fixture();
        scan_crate(&root.join("src/lib.rs"), "modtier", &root)
    }

    /// **The scan runs against real source, not a snippet.** A hand-written
    /// string agrees with whatever the scanner happens to do.
    #[test]
    fn every_module_in_the_fixture_is_found() {
        let m = scan_fixture();
        let found: Vec<String> = m.modules.iter().map(|m| m.to_string()).collect();
        for want in [
            "modtier",
            "modtier::parse",
            "modtier::exec",
            "modtier::shared",
        ] {
            assert!(
                found.contains(&want.to_string()),
                "missing {want}: {found:?}"
            );
        }
    }

    /// The forbidden call is found, resolved to the right item, and carries
    /// the file and line a reader needs to go and look.
    #[test]
    fn the_forbidden_call_resolves_to_the_function_it_names() {
        let m = scan_fixture();
        let call = m
            .references
            .iter()
            .find(|r| {
                matches!(&r.destination, Destination::Definite(d)
                    if d.module.to_string() == "modtier::exec" && d.name == "run")
            })
            .unwrap_or_else(|| panic!("{:#?}", m.references));
        assert_eq!(call.origin.module.to_string(), "modtier::parse");
        assert_eq!(call.origin.name, "parse");
        assert!(call.span.file.ends_with("parse.rs"), "{:?}", call.span);
        assert!(
            call.span.line > 0,
            "a line the reader can jump to: {:?}",
            call.span
        );
    }

    /// The permitted calls resolve too -- a scan that only found the
    /// violation would be finding it by luck.
    #[test]
    fn the_permitted_calls_resolve_as_well() {
        let m = scan_fixture();
        let shared: Vec<_> = m
            .references
            .iter()
            .filter(|r| {
                matches!(&r.destination, Destination::Definite(d)
                    if d.module.to_string() == "modtier::shared" && d.name == "normalise")
            })
            .collect();
        assert_eq!(shared.len(), 2, "both sides call it: {:#?}", m.references);
    }

    /// What the scan could not follow has to be *said*, or the report
    /// reads as complete when it is not. The bucket's own methods are
    /// standard-library calls this scan does not resolve, and the module
    /// holding them must say so.
    #[test]
    fn method_calls_the_scan_cannot_follow_are_reported_once_for_the_module() {
        let m = scan_fixture();
        let gap = m
            .gaps
            .iter()
            .find(|g| matches!(&g.scope, GapScope::Module(x) if x.to_string() == "modtier::shared"))
            .unwrap_or_else(|| panic!("{:#?}", m.gaps));
        assert!(gap.cause.contains("method call"), "{}", gap.cause);
        assert!(
            gap.cause.contains('2'),
            "how many, so a reader can judge the size of the hole: {}",
            gap.cause
        );
    }

    /// A bare call names a function in the module doing the calling, not
    /// one at the crate root. Getting this backwards attributes a call to
    /// the wrong component, which is the one mistake this tier cannot
    /// afford.
    #[test]
    fn a_bare_call_resolves_inside_the_calling_module_not_the_crate_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub mod inner;\npub fn helper() {}",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/inner.rs"),
            "pub fn helper() {}\npub fn go() { helper(); }",
        )
        .unwrap();
        let m = scan_crate(&dir.path().join("src/lib.rs"), "k", dir.path());
        let call = m
            .references
            .iter()
            .find(|r| r.origin.name == "go")
            .unwrap_or_else(|| panic!("{:#?}", m.references));
        match &call.destination {
            Destination::Definite(d) => assert_eq!(
                d.module.to_string(),
                "k::inner",
                "the one next to it, not the crate root's"
            ),
            other => panic!("{other:?}"),
        }
    }

    /// A glob import brings in names this scan cannot enumerate, so a bare
    /// name afterwards might come from anywhere. Resolving it anyway would
    /// invent a destination, and an invented destination is how a clean
    /// report gets written about code nobody read.
    #[test]
    fn a_name_that_could_have_come_from_a_glob_is_not_resolved_across_modules() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub mod other;\npub mod here;",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/other.rs"), "pub fn target() {}").unwrap();
        std::fs::write(
            dir.path().join("src/here.rs"),
            "use crate::other::*;\npub fn go() { target(); }",
        )
        .unwrap();
        let m = scan_crate(&dir.path().join("src/lib.rs"), "k", dir.path());
        let call = m
            .references
            .iter()
            .find(|r| r.origin.name == "go")
            .unwrap_or_else(|| panic!("{:#?}", m.references));
        assert!(
            !matches!(&call.destination, Destination::Definite(d)
                if d.module.to_string() == "k::other"),
            "a glob's contents are not known, so this must not resolve there: {:?}",
            call.destination
        );
    }

    /// A path this scan cannot place stays unresolved with a reason, and
    /// never becomes a definite destination.
    #[test]
    fn an_unplaceable_path_is_refused_rather_than_guessed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn go() { crate::nowhere::gone(); }",
        )
        .unwrap();
        let m = scan_crate(&dir.path().join("src/lib.rs"), "k", dir.path());
        let r = &m.references[0];
        match &r.destination {
            Destination::Unresolved { reason } => {
                assert!(reason.contains("nowhere"), "{reason}")
            }
            other => panic!("must not resolve: {other:?}"),
        }
    }

    /// A module declared but not found leaves a gap naming it, so a rule
    /// about it cannot come back clean.
    #[test]
    fn a_module_whose_file_is_missing_becomes_a_named_gap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub mod ghost;").unwrap();
        let m = scan_crate(&dir.path().join("src/lib.rs"), "k", dir.path());
        assert!(
            m.gaps.iter().any(|g| matches!(&g.scope, GapScope::Module(x)
                if x.to_string() == "k::ghost")),
            "{:#?}",
            m.gaps
        );
    }

    /// A `#[path]` module is read where it says or not at all -- never by
    /// trying the conventional filename instead.
    #[test]
    fn a_path_attribute_module_is_declared_unread_rather_than_guessed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "#[path = \"elsewhere.rs\"]\npub mod here;",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/here.rs"), "pub fn trap() {}").unwrap();
        let m = scan_crate(&dir.path().join("src/lib.rs"), "k", dir.path());
        assert!(
            m.gaps.iter().any(|g| g.cause.contains("#[path]")),
            "{:#?}",
            m.gaps
        );
        assert!(
            !m.items.iter().any(|i| i.id.name == "trap"),
            "the conventional file must not be read instead"
        );
    }
}
