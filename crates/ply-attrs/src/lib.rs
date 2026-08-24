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
use syn::{parse_macro_input, Expr, ItemFn};

/// `#[ply::requires(expr)]` — precondition: what must hold on entry.
#[proc_macro_attribute]
pub fn requires(attr: TokenStream, item: TokenStream) -> TokenStream {
    let expr = parse_macro_input!(attr as Expr);
    let func = parse_macro_input!(item as ItemFn);

    let ItemFn { attrs, vis, sig, block } = func;

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

    let ItemFn { attrs, vis, sig, block } = func;

    quote! {
        #(#attrs)*
        #[cfg_attr(kani, kani::ensures(#closure))]
        #vis #sig #block
    }
    .into()
}
