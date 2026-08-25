//! `ply-attrs` — the contract attribute macros (D2).
//!
//! `#[ply::requires(expr)]` and `#[ply::ensures(|result| expr)]` re-emit the
//! annotated item **unchanged**, plus a `#[cfg_attr(kani, kani::requires(...))]`
//! (or the `ensures` equivalent). Under plain `cargo build`/`cargo test` the
//! `cfg_attr` is inert — the function compiles exactly as written. Under
//! `cargo kani` (which sets `--cfg kani`), the attribute instruments the real
//! function so Kani's `#[kani::proof_for_contract]` can verify it.
//!
//! Fixtures depend on this crate renamed to `ply` (`ply = { package =
//! "ply-attrs", path = "..." }`) so the attribute path in source reads exactly
//! `#[ply::requires(...)]`/`#[ply::ensures(...)]`, matching D2 verbatim.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Expr, ItemFn, parse_macro_input};

/// `#[ply::requires(expr)]` — precondition: what must hold on entry.
#[proc_macro_attribute]
pub fn requires(attr: TokenStream, item: TokenStream) -> TokenStream {
    let expr = parse_macro_input!(attr as Expr);
    let func = parse_macro_input!(item as ItemFn);

    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = func;

    quote! {
        #(#attrs)*
        #[cfg_attr(kani, kani::requires(#expr))]
        #vis #sig #block
    }
    .into()
}

/// `#[ply::ensures(|result| expr)]` — postcondition: what the function
/// guarantees about its result. The argument is the same `|result| expr`
/// closure shape Kani's own `kani::ensures` expects, so it re-emits verbatim.
#[proc_macro_attribute]
pub fn ensures(attr: TokenStream, item: TokenStream) -> TokenStream {
    let closure = parse_macro_input!(attr as Expr);
    let func = parse_macro_input!(item as ItemFn);

    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = func;

    quote! {
        #(#attrs)*
        #[cfg_attr(kani, kani::ensures(#closure))]
        #vis #sig #block
    }
    .into()
}

/// `ply::unresolved!(id, "note")` — a decision nobody has made yet (§5.6).
///
/// It expands to `unimplemented!("unresolved #<id>: <note>")` **always**,
/// dev and prod alike: simple, honest, greppable. A marker that compiled
/// away in release would be a gap that ships silently, which is the one
/// thing this construct exists to prevent. `cargo ply worklist` lists every
/// marker with its span, its enclosing function and what it blocks.
#[proc_macro]
pub fn unresolved(input: TokenStream) -> TokenStream {
    expand_unresolved(input.into()).into()
}

/// The expansion, over `proc_macro2` tokens so it can be tested from
/// inside this crate (a proc-macro crate cannot invoke its own macros).
///
/// Arguments Ply cannot read are passed through as written rather than
/// rejected: the macro's job is to make the gap panic and be greppable, and
/// a compile error about the shape of a TODO helps nobody.
fn expand_unresolved(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let args: Vec<proc_macro2::TokenTree> = input.into_iter().collect();
    let mut id = String::new();
    let mut note = String::new();
    for tree in &args {
        if let proc_macro2::TokenTree::Literal(lit) = tree {
            let text = lit.to_string();
            if text.starts_with('"') {
                if note.is_empty() {
                    note = text.trim_matches('"').to_string();
                }
            } else if id.is_empty() {
                id = text;
            }
        }
    }
    let message = if note.is_empty() {
        format!("unresolved #{id}")
    } else {
        format!("unresolved #{id}: {note}")
    };
    quote! { unimplemented!(#message) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §5.6, verbatim: "It expands to `unimplemented!("unresolved #147:
    /// employee discount undecided")` — always, dev and prod alike."
    ///
    /// The expansion is the whole feature. A marker that compiled away in
    /// release would be a decision nobody made, shipping silently.
    #[test]
    fn a_marker_expands_to_an_unconditional_unimplemented_naming_the_id_and_the_note() {
        let tokens: proc_macro2::TokenStream =
            "147, \"employee discount undecided\"".parse().unwrap();
        assert_eq!(
            expand_unresolved(tokens).to_string(),
            "unimplemented ! (\"unresolved #147: employee discount undecided\")"
        );
    }
}
