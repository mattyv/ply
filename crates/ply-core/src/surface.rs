//! What a listing command can read out of a crate's own source: unresolved
//! markers (§5.6), profile escapes (§5.3), derived bodies (§5.7), and the
//! helper functions a contract calls (§5.4a).
//!
//! These are the constructs `cargo ply audit` and `cargo ply worklist`
//! report, and they are all *declarations a human wrote*, not findings an
//! engine produced. Reading them needs one walk over `<crate>/src/**/*.rs`
//! and no engine at all, which is why both commands are fast.
//!
//! **Three of the four constructs have no macro behind them yet.**
//! `#[ply::allow(...)]` belongs to the always-on architecture tier (M2) and
//! `#[ply::derived(...)]` is written by `cargo ply synth` (M6); neither
//! macro exists in `ply-attrs`, so source carrying one does not compile
//! today. The scanner reads them anyway — when the macro lands, the listing
//! is already there, and until then the commands say plainly that they
//! found none. `ply::unresolved!` does exist (§5.6).

use std::path::{Path, PathBuf};

use syn::spanned::Spanned;

/// Everything one scan of a crate's source found.
#[derive(Debug, Default)]
pub struct SourceSurface {
    pub markers: Vec<Marker>,
    pub escapes: Vec<Escape>,
    pub derived: Vec<Derived>,
}

/// One `ply::unresolved!(id, "note")` in the code (§5.6): a decision nobody
/// has made yet, standing where the code that implements it would go.
#[derive(Debug, Clone)]
pub struct Marker {
    /// The id, as written. `None` when the macro was called with something
    /// other than an integer literal first — Ply reports the marker it can
    /// see rather than dropping it for being malformed.
    pub id: Option<u64>,
    pub note: Option<String>,
    /// The function the marker sits in, as the path a reader would type
    /// (`pricing::quote`). `None` for a marker outside any function.
    pub enclosing_fn: Option<String>,
    /// Relative to the crate directory, so output is stable across machines.
    pub file: String,
    pub line: usize,
    pub col: usize,
}

/// One `#[ply::allow(name, reason = "...")]` (§5.3): a ban or an item-tier
/// rule switched off for one item, on purpose, by somebody.
#[derive(Debug, Clone)]
pub struct Escape {
    /// The item the escape sits on, as the path a reader would type.
    pub item: String,
    /// The ban name or diagnostic code it suppresses.
    pub suppressed: String,
    pub reason: Option<String>,
    pub file: String,
    pub line: usize,
}

/// One `#[ply::derived(spec_hash = "...")]` (§5.7): a body the model wrote
/// against its spec.
#[derive(Debug, Clone)]
pub struct Derived {
    pub item: String,
    pub spec_hash: Option<String>,
    pub file: String,
    pub line: usize,
}

/// Walks every `.rs` file under `<crate_dir>/src`, in path order.
///
/// A file that will not parse is skipped rather than reported: this is a
/// listing pass, and `check`/`verify` are what report a source file Ply
/// cannot read.
pub fn scan_crate(crate_dir: &Path) -> SourceSurface {
    let mut out = SourceSurface::default();
    let mut files = Vec::new();
    collect_rs_files(&crate_dir.join("src"), &mut files);
    files.sort();
    for path in files {
        let rel = path
            .strip_prefix(crate_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = syn::parse_file(&text) else {
            continue;
        };
        scan_items(&file.items, "", &rel, &mut out);
    }
    out
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn scan_items(items: &[syn::Item], prefix: &str, file: &str, out: &mut SourceSurface) {
    for item in items {
        match item {
            syn::Item::Fn(f) => {
                let name = qualify(prefix, &f.sig.ident.to_string());
                scan_attrs(&f.attrs, &name, file, out);
                scan_block(&f.block, &name, file, out);
            }
            syn::Item::Mod(m) => {
                let name = qualify(prefix, &m.ident.to_string());
                scan_attrs(&m.attrs, &name, file, out);
                if let Some((_, items)) = &m.content {
                    scan_items(items, &name, file, out);
                }
            }
            syn::Item::Impl(i) => {
                // An `impl` block's methods are named `Type::method`, the
                // same spelling a fn claim uses (§5.1a rule 3).
                let ty = quote::ToTokens::to_token_stream(&i.self_ty).to_string();
                let ty = ty.replace(' ', "");
                for sub in &i.items {
                    if let syn::ImplItem::Fn(f) = sub {
                        let name = qualify(prefix, &format!("{ty}::{}", f.sig.ident));
                        scan_attrs(&f.attrs, &name, file, out);
                        scan_block(&f.block, &name, file, out);
                    }
                }
            }
            other => {
                if let Some(attrs) = item_attrs(other) {
                    scan_attrs(attrs, &qualify(prefix, &item_name(other)), file, out);
                }
            }
        }
    }
}

fn qualify(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}::{name}")
    }
}

fn item_attrs(item: &syn::Item) -> Option<&Vec<syn::Attribute>> {
    match item {
        syn::Item::Const(i) => Some(&i.attrs),
        syn::Item::Enum(i) => Some(&i.attrs),
        syn::Item::Static(i) => Some(&i.attrs),
        syn::Item::Struct(i) => Some(&i.attrs),
        syn::Item::Trait(i) => Some(&i.attrs),
        syn::Item::Type(i) => Some(&i.attrs),
        _ => None,
    }
}

fn item_name(item: &syn::Item) -> String {
    match item {
        syn::Item::Const(i) => i.ident.to_string(),
        syn::Item::Enum(i) => i.ident.to_string(),
        syn::Item::Static(i) => i.ident.to_string(),
        syn::Item::Struct(i) => i.ident.to_string(),
        syn::Item::Trait(i) => i.ident.to_string(),
        syn::Item::Type(i) => i.ident.to_string(),
        _ => String::new(),
    }
}

/// `#[ply::allow(...)]` and `#[ply::derived(...)]` on one item.
fn scan_attrs(attrs: &[syn::Attribute], item: &str, file: &str, out: &mut SourceSurface) {
    for attr in attrs {
        let path = path_string(attr.path());
        let line = attr.span().start().line;
        match path.as_str() {
            "ply::allow" => {
                let (first, reason) = attr_args(attr);
                out.escapes.push(Escape {
                    item: item.to_string(),
                    suppressed: first.unwrap_or_default(),
                    reason,
                    file: file.to_string(),
                    line,
                });
            }
            "ply::derived" => {
                let (_, spec_hash) = attr_args(attr);
                out.derived.push(Derived {
                    item: item.to_string(),
                    spec_hash,
                    file: file.to_string(),
                    line,
                });
            }
            _ => {}
        }
    }
}

/// The bare word an attribute leads with (`no_panics`, `A0402`) and the
/// string value of its single `name = "..."` argument (`reason`,
/// `spec_hash`), whichever it carries.
fn attr_args(attr: &syn::Attribute) -> (Option<String>, Option<String>) {
    let mut word = None;
    let mut value = None;
    let parsed = attr.parse_args_with(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
    );
    let Ok(metas) = parsed else {
        return (word, value);
    };
    for meta in metas {
        match meta {
            syn::Meta::Path(p) if word.is_none() => word = Some(path_string(&p)),
            syn::Meta::NameValue(nv) => {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
                {
                    value = Some(s.value());
                }
            }
            _ => {}
        }
    }
    (word, value)
}

/// `ply::unresolved!(id, "note")` anywhere inside one function body.
fn scan_block(block: &syn::Block, enclosing_fn: &str, file: &str, out: &mut SourceSurface) {
    struct Visitor<'a> {
        enclosing_fn: &'a str,
        file: &'a str,
        markers: Vec<Marker>,
    }
    impl syn::visit::Visit<'_> for Visitor<'_> {
        fn visit_macro(&mut self, mac: &syn::Macro) {
            let path = path_string(&mac.path);
            // `ply::unresolved!` as §5.6 writes it, and the bare form a
            // `use ply::unresolved;` makes natural. Nothing else: a
            // project's own `unresolved!` macro under some other path is
            // not Ply's marker.
            if path != "ply::unresolved" && path != "unresolved" {
                return;
            }
            let start = mac.path.span().start();
            let (id, note) = marker_args(mac);
            self.markers.push(Marker {
                id,
                note,
                enclosing_fn: Some(self.enclosing_fn.to_string()),
                file: self.file.to_string(),
                line: start.line,
                col: start.column + 1,
            });
        }
    }
    let mut v = Visitor {
        enclosing_fn,
        file,
        markers: Vec::new(),
    };
    syn::visit::Visit::visit_block(&mut v, block);
    out.markers.extend(v.markers);
}

fn marker_args(mac: &syn::Macro) -> (Option<u64>, Option<String>) {
    let parsed = mac.parse_body_with(
        syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
    );
    let Ok(args) = parsed else {
        return (None, None);
    };
    let mut id = None;
    let mut note = None;
    for arg in args {
        if let syn::Expr::Lit(syn::ExprLit { lit, .. }) = arg {
            match lit {
                syn::Lit::Int(i) if id.is_none() => id = i.base10_parse::<u64>().ok(),
                syn::Lit::Str(s) if note.is_none() => note = Some(s.value()),
                _ => {}
            }
        }
    }
    (id, note)
}

fn path_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Whether two contract clauses are the same expression, spelled two ways.
///
/// A clause can be written twice — once as a `#[ply::requires]` attribute
/// and once in `ply.yaml` — and the two strings do not match: the attribute
/// comes back from the parser token-spaced (`bps_ok (bps)`) while the
/// document holds what the user typed. Comparing raw strings made `audit`
/// report one environmental assumption as two, which is a trust surface
/// overstating itself.
///
/// The comparison is over parsed tokens, not over whitespace-stripped text,
/// so a difference inside a string literal stays a difference. When either
/// side does not parse, the strings themselves decide — two clauses Ply
/// cannot read are not thereby the same clause.
pub fn same_expression(a: &str, b: &str) -> bool {
    match (
        syn::parse_str::<syn::Expr>(a),
        syn::parse_str::<syn::Expr>(b),
    ) {
        (Ok(ea), Ok(eb)) => {
            use quote::ToTokens;
            ea.to_token_stream().to_string() == eb.to_token_stream().to_string()
        }
        _ => a == b,
    }
}

/// The helper functions one contract expression calls (§5.4a).
///
/// Two names that look like calls are deliberately not helpers: `old(expr)`
/// is §5.4a's own two-state primitive, and a capitalised path is a type or
/// enum-variant constructor (`Some(x)`, `Ok(v)`) — §5.5 draws that same
/// line for call sites, and for the same reason: listing `Some` as a
/// trusted helper would put noise on the one surface that has to stay
/// readable.
///
/// An expression Ply cannot parse yields nothing. The contract subset is
/// enforced elsewhere (`E0501`); a listing command falling over on a
/// document `verify` reads happily would be the worse failure.
pub fn contract_helpers(expr_src: &str) -> Vec<String> {
    let Ok(expr) = syn::parse_str::<syn::Expr>(expr_src) else {
        return vec![];
    };
    struct Visitor {
        names: Vec<String>,
    }
    impl syn::visit::Visit<'_> for Visitor {
        fn visit_expr_call(&mut self, call: &syn::ExprCall) {
            if let syn::Expr::Path(p) = &*call.func {
                let name = path_string(&p.path);
                let last = name.rsplit("::").next().unwrap_or(&name);
                let is_constructor = last.chars().next().is_some_and(|c| c.is_uppercase());
                if !is_constructor && last != "old" && !self.names.contains(&name) {
                    self.names.push(name);
                }
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let mut v = Visitor { names: Vec::new() };
    syn::visit::Visit::visit_expr(&mut v, &expr);
    v.names
}
