//! Finite lowering of collection quantifiers for the stack-backed byte domain.
//! Only direct shared-byte iterators and zero-based, bounded usize ranges are
//! expanded. Other expressions keep their original Rust semantics and code.
use std::collections::BTreeMap;

use quote::quote;
use syn::visit::{self, Visit};
use syn::visit_mut::{self, VisitMut};
use syn::{Expr, ExprClosure, Pat, RangeLimits};

use crate::harness::{Param, RustType};

#[derive(Clone)]
enum Domain {
    Slice(RustType),
    Index,
}

struct ConstantIndex<'a> {
    name: &'a str,
    value: Expr,
}
impl VisitMut for ConstantIndex<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if ident(expr).as_deref() == Some(self.name) {
            *expr = self.value.clone();
            return;
        }
        match expr {
            Expr::Block(_)
            | Expr::Macro(_)
            | Expr::Match(_)
            | Expr::ForLoop(_)
            | Expr::If(_)
            | Expr::While(_)
            | Expr::Loop(_) => return,
            Expr::Closure(c)
                if c.inputs
                    .iter()
                    .any(|p| binding(p).is_none() || binding(p).as_deref() == Some(self.name)) =>
            {
                return;
            }
            _ => {}
        }
        visit_mut::visit_expr_mut(self, expr);
    }
}

struct CannotInline<'a> {
    name: &'a str,
    found: bool,
}
impl<'ast> Visit<'ast> for CannotInline<'_> {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        if ident(expr).as_deref() == Some(self.name)
            || matches!(
                expr,
                Expr::Return(_) | Expr::Try(_) | Expr::Await(_) | Expr::Yield(_) | Expr::Macro(_)
            )
        {
            self.found = true;
        }
        visit::visit_expr(self, expr);
    }
}

fn bare(expr: &Expr) -> &Expr {
    match expr {
        Expr::Paren(e) => bare(&e.expr),
        Expr::Group(e) => bare(&e.expr),
        _ => expr,
    }
}

fn ident(expr: &Expr) -> Option<String> {
    let Expr::Path(e) = bare(expr) else {
        return None;
    };
    e.path.get_ident().map(ToString::to_string)
}

fn binding(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Ident(p) if p.subpat.is_none() && p.mutability.is_none() && p.by_ref.is_none() => {
            Some(p.ident.to_string())
        }
        Pat::Type(p) => binding(&p.pat),
        _ => None,
    }
}

fn bounded_end(expr: &Expr, domains: &BTreeMap<String, Domain>) -> bool {
    match bare(expr) {
        Expr::Path(_) => {
            ident(expr).is_some_and(|n| matches!(domains.get(&n), Some(Domain::Index)))
        }
        Expr::MethodCall(e) if e.method == "len" && e.args.is_empty() => {
            ident(&e.receiver).is_some_and(|n| matches!(domains.get(&n), Some(Domain::Slice(_))))
        }
        _ => false,
    }
}

fn lower(expr: &Expr, domains: &BTreeMap<String, Domain>, k: u32) -> Expr {
    // Do not descend into arbitrary binding scopes, calls, macros or closures.
    // This keeps the syntactic domain map from mistaking a shadowed name for
    // a parameter, and leaves unsupported iterator forms untouched.
    let mut out = expr.clone();
    match &mut out {
        Expr::Binary(e) => {
            *e.left = lower(&e.left, domains, k);
            *e.right = lower(&e.right, domains, k);
        }
        Expr::Unary(e) => *e.expr = lower(&e.expr, domains, k),
        Expr::Paren(e) => *e.expr = lower(&e.expr, domains, k),
        Expr::Group(e) => *e.expr = lower(&e.expr, domains, k),
        Expr::MethodCall(all) if all.method == "all" && all.args.len() == 1 => {
            let Expr::Closure(predicate) = bare(&all.args[0]) else {
                return out;
            };
            if predicate.capture.is_some()
                || predicate.asyncness.is_some()
                || predicate.inputs.len() != 1
            {
                return out;
            }
            let Some(name) = binding(&predicate.inputs[0]) else {
                return out;
            };
            let (end, receiver, element_ty, domain) = match bare(&all.receiver) {
                Expr::MethodCall(iter) if iter.method == "iter" && iter.args.is_empty() => {
                    let Some(collection) = ident(&iter.receiver) else {
                        return out;
                    };
                    let Some(Domain::Slice(RustType::Slice(inner))) = domains.get(&collection)
                    else {
                        return out;
                    };
                    let ty: syn::Type = match inner.as_ref() {
                        RustType::Slice(byte) if byte.as_ref() == &RustType::U8 => {
                            syn::parse_quote!(&&[u8])
                        }
                        RustType::U8 => syn::parse_quote!(&u8),
                        _ => return out,
                    };
                    let receiver = (*iter.receiver).clone();
                    let end: Expr = syn::parse_quote!((#receiver).len());
                    (end, Some(receiver), ty, Domain::Slice(*inner.clone()))
                }
                Expr::Range(range) if matches!(range.limits, RangeLimits::HalfOpen(_)) => {
                    let (Some(start), Some(end)) = (&range.start, &range.end) else {
                        return out;
                    };
                    if !matches!(bare(start), Expr::Lit(lit) if matches!(&lit.lit, syn::Lit::Int(n) if n.base10_digits() == "0"))
                        || !(bounded_end(end, domains)
                            || matches!(bare(end), Expr::Lit(lit) if matches!(&lit.lit, syn::Lit::Int(n) if n.suffix() == "usize" && n.base10_parse::<u32>().is_ok_and(|n| n <= k))))
                    {
                        return out;
                    }
                    (
                        (**end).clone(),
                        None,
                        syn::parse_quote!(usize),
                        Domain::Index,
                    )
                }
                _ => return out,
            };
            let mut nested = domains.clone();
            nested.insert(name.clone(), domain.clone());
            let mut predicate = predicate.clone();

            if !matches!(predicate.inputs[0], Pat::Type(_)) {
                predicate.inputs[0] = Pat::Type(syn::PatType {
                    attrs: Vec::new(),
                    pat: Box::new(predicate.inputs[0].clone()),
                    colon_token: Default::default(),
                    ty: Box::new(element_ty),
                });
            }
            let mut terms = Vec::new();
            for i in 0..k {
                if let Expr::Lit(lit) = bare(&end)
                    && let syn::Lit::Int(n) = &lit.lit
                    && n.base10_parse::<u32>().is_ok_and(|n| i >= n)
                {
                    continue;
                }
                let i = syn::Index::from(i as usize);
                let literal =
                    syn::LitInt::new(&format!("{}usize", i.index), proc_macro2::Span::call_site());
                let index: Expr = syn::parse_quote!(#literal);
                let mut predicate = predicate.clone();
                if matches!(domain, Domain::Index) {
                    // Static indices let CBMC avoid symbolic iterator state.
                    ConstantIndex {
                        name: &name,
                        value: index.clone(),
                    }
                    .visit_expr_mut(&mut predicate.body);
                }
                let value: Expr = match &receiver {
                    Some(receiver) => syn::parse_quote!(&(#receiver)[#index]),
                    None => index.clone(),
                };
                if receiver.is_some() {
                    ConstantIndex {
                        name: &name,
                        value: value.clone(),
                    }
                    .visit_expr_mut(&mut predicate.body);
                }
                *predicate.body = lower(&predicate.body, &nested, k);
                let mut cannot_inline = CannotInline {
                    name: &name,
                    found: false,
                };
                cannot_inline.visit_expr(&predicate.body);
                let body = &predicate.body;
                if !cannot_inline.found {
                    terms.push(quote!((#index >= (#end) || (#body))));
                } else {
                    terms.push(quote!((#index >= (#end) || (#predicate)(#value))));
                }
            }
            out = syn::parse2(quote!((true #(&& #terms)*)))
                .expect("finite all expression is valid Rust");
        }
        _ => {}
    }
    out
}

pub(crate) fn lower_byte_ensures(closure: &ExprClosure, params: &[Param], k: u32) -> ExprClosure {
    let mut domains: BTreeMap<String, Domain> = params
        .iter()
        .filter(|p| p.ty.is_nested_byte_slices())
        .map(|p| (p.name.clone(), Domain::Slice(p.ty.clone())))
        .collect();
    for pat in &closure.inputs {
        if let Some(name) = binding(pat) {
            domains.remove(&name);
        }
    }
    let mut out = closure.clone();
    out.body = Box::new(lower(&out.body, &domains, k));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn params() -> Vec<Param> {
        vec![Param {
            name: "keys".into(),
            ty: RustType::Slice(Box::new(RustType::Slice(Box::new(RustType::U8)))),
            by_ref: true,
        }]
    }

    #[test]
    fn finite_contract_matches_rust_for_empty_duplicate_and_arbitrary_bytes() {
        let original: ExprClosure = syn::parse_quote!(|result: &bool| *result
            == (!keys.is_empty()
                && keys.iter().all(|key| !key.is_empty())
                && (0..keys.len()).all(|i| (0..i).all(|j| keys[i] != keys[j]))));
        let lowered = lower_byte_ensures(&original, &params(), 3);
        let macro_original: ExprClosure =
            syn::parse_quote!(|result: &bool| (0..keys.len()).all(|i| matches!(i, 0)));
        let macro_lowered = lower_byte_ensures(&macro_original, &params(), 3);
        let type_original: ExprClosure = syn::parse_quote!(|result: &bool| (0..keys.len())
            .all(|i| std::mem::size_of_val(&i) == std::mem::size_of::<usize>()));
        let type_lowered = lower_byte_ensures(&type_original, &params(), 3);
        let shadow_original: ExprClosure = syn::parse_quote!(
            |result: &bool| (0..keys.len()).all(|i| (|(i,): (usize,)| i == 7)((7,)))
        );
        let shadow_lowered = lower_byte_ensures(&shadow_original, &params(), 3);
        let dir = tempfile::tempdir().unwrap();
        let source = format!(
            r#"
fn compare(keys: &[&[u8]]) {{
    let i = 2usize;
    for result in [false, true] {{
        assert_eq!(({type_original})(&result), ({type_lowered})(&result));
        assert_eq!(({macro_original})(&result), ({macro_lowered})(&result));
        assert_eq!(({shadow_original})(&result), ({shadow_lowered})(&result));
        assert_eq!(({original})(&result), ({lowered})(&result), "{{:?}}", keys);
    }}
}}
fn main() {{
    let mut rows = vec![vec![]];
    for len in 1..=3 {{
        for bits in 0..(1usize << len) {{
            rows.push((0..len).map(|i| if bits & (1 << i) == 0 {{ 0 }} else {{ 255 }}).collect());
        }}
    }}
    compare(&[]);
    for a in &rows {{
        compare(&[a]);
        for b in &rows {{
            compare(&[a, b]);
            for c in &rows {{ compare(&[a, b, c]); }}
        }}
    }}
}}
"#,
            original = original.to_token_stream(),
            lowered = lowered.to_token_stream(),
            type_original = type_original.to_token_stream(),
            type_lowered = type_lowered.to_token_stream(),
            macro_original = macro_original.to_token_stream(),
            macro_lowered = macro_lowered.to_token_stream(),
            shadow_original = shadow_original.to_token_stream(),
            shadow_lowered = shadow_lowered.to_token_stream()
        );
        let path = dir.path().join("equivalence.rs");
        std::fs::write(&path, source).unwrap();
        let binary = dir.path().join("equivalence");
        let output = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg("-Awarnings")
            .arg(&path)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            std::process::Command::new(binary)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn unknown_binding_scopes_and_shadowed_parameters_keep_original_code() {
        for text in [
            "|keys| keys.iter().all(|key| !key.is_empty())",
            "|result| { let keys = &[true; 9]; keys.iter().all(|key| *key) }",
            "|result| (0..9).all(|i| i < 9)",
            "|result| (0..3).all(|i| std::mem::size_of_val(&i) == 4)",
            "|result| keys.iter().all(move |key| !key.is_empty())",
        ] {
            let original: ExprClosure = syn::parse_str(text).unwrap();
            assert_eq!(lower_byte_ensures(&original, &params(), 3), original);
        }
    }

    #[test]
    fn zero_bound_has_no_index_or_callback_access() {
        let original: ExprClosure =
            syn::parse_quote!(|result| keys.iter().all(|key| !key.is_empty()));
        let lowered = lower_byte_ensures(&original, &params(), 0);
        assert_eq!(lowered.body.to_token_stream().to_string(), "(true)");
    }
}
