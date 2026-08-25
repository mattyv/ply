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
//! Resolution reaches two places and no further: top-level `fn` items in
//! the caller's own file, and top-level (or `mod`-nested) `fn` items in a
//! **path dependency's** `src/lib.rs`. A call into `std`, `core`, or a
//! registry crate is `Unresolved` — outside this rule's reach in v1, and
//! said so in §5.5 rather than left for a user to discover.

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
        signature: CalleeSignature,
    },
    /// Resolved, and nothing anywhere describes it: §5.5's third branch.
    /// Ply refuses to descend.
    Unclaimed,
    /// Ply cannot see this callee's source at all (`std`, `core`, a
    /// registry crate). Outside this rule's reach in v1 — recorded in §5.5,
    /// not silently treated as claimed.
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
/// own file, the crate directory (for path-dependency lookup), and the
/// contracts `ply.yaml` declares.
pub struct Resolver {
    local: syn::File,
    crate_dir: PathBuf,
    declared: BTreeMap<String, DeclaredContract>,
    dep_sources: BTreeMap<String, syn::File>,
}

impl Resolver {
    /// `lib_src` is the caller's own file source; `declared` is keyed by the
    /// path a caller writes.
    pub fn new(
        lib_src: &str,
        crate_dir: &Path,
        declared: BTreeMap<String, DeclaredContract>,
    ) -> anyhow::Result<Resolver> {
        Ok(Resolver {
            local: syn::parse_file(lib_src)?,
            crate_dir: crate_dir.to_path_buf(),
            declared,
            dep_sources: BTreeMap::new(),
        })
    }

    pub fn classify(&mut self, site: &CallSite) -> ResolvedCall {
        let status = self.status_of(&site.path);
        ResolvedCall {
            site: site.clone(),
            status,
        }
    }

    fn status_of(&mut self, path: &str) -> CalleeStatus {
        let segments: Vec<&str> = path.split("::").collect();
        let found = if segments.len() == 1 {
            find_fn(&self.local, &segments)
        } else {
            match self.dep_file(segments[0]) {
                Some(file) => find_fn(file, &segments[1..]),
                // A multi-segment path may still be local
                // (`self::helper`, an inline `mod`): try the caller's own
                // file before giving up.
                None => find_fn(&self.local, &segments),
            }
        };
        let Some(f) = found else {
            return CalleeStatus::Unresolved;
        };
        if has_inline_contract(&f) {
            return CalleeStatus::Contracted;
        }
        match self.declared.get(path) {
            Some(contract) => CalleeStatus::Assumed {
                contract: contract.clone(),
                signature: signature_of(&f),
            },
            None => CalleeStatus::Unclaimed,
        }
    }

    /// Parses (once) the `src/lib.rs` of the path dependency registered
    /// under `dep_name` in the crate's `Cargo.toml`. `dep_name` is the key
    /// in `[dependencies]`, which is also the first segment a caller writes
    /// — including when the key renames the package
    /// (`ledger = { package = "ply-vetting-004-ledger", path = "../legacy" }`).
    fn dep_file(&mut self, dep_name: &str) -> Option<&syn::File> {
        if !self.dep_sources.contains_key(dep_name) {
            let cargo_toml = std::fs::read_to_string(self.crate_dir.join("Cargo.toml")).ok()?;
            let rel = path_dependency(&cargo_toml, dep_name)?;
            let lib = self.crate_dir.join(rel).join("src/lib.rs");
            let src = std::fs::read_to_string(lib).ok()?;
            let parsed = syn::parse_file(&src).ok()?;
            self.dep_sources.insert(dep_name.to_string(), parsed);
        }
        self.dep_sources.get(dep_name)
    }
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
/// top-level `fn` with the last segment's name.
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
                signature,
            } => {
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
