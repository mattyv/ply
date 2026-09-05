//! The D7 replayable-test renderer (docs/plans/d7-replayable-tests.md):
//! turns a §5.4a `ensures` AST plus a concrete witness into a plain,
//! overflow-safe `#[test]` that asserts the postcondition *explicitly* --
//! the only artifact that can go red for an `ensures` violation, since
//! Kani's own playback replays the body only (ADR-0003 caveat 3).
//!
//! Scope note: this slice's fixtures only ever exercise a top-level
//! comparison (`*result == x`, `*result <= ...`) -- so that is the one case
//! rendered with a value-naming message. Anything else (top-level `&&`/`||`,
//! or an operator other than a comparison) still renders and still fails
//! for the right reason, but with the generic fallback message the D7 plan
//! prescribes for exactly this case, not a per-case invention.

use anyhow::{Result, bail};
use quote::ToTokens;
use syn::{BinOp, Expr, ExprClosure};

use crate::engines::kani::WitnessValue;
use crate::harness::{ContractFn, RustType};

/// One `old(expr)` occurrence lifted out of an `ensures` clause: the name
/// of the binding a generated harness must create, and the expression it
/// must read into it.
///
/// `old(expr)` means "the value `expr` had when the function was entered"
/// (§5.4a). The model checker has a primitive for that and Kani maps the
/// call onto it; a generated `#[test]`/proptest harness has none, so the
/// spec prescribes the only thing that can work there -- "evaluate `expr`
/// before the call and substitute the snapshot". Until 2026-08-25 nothing
/// did, and the clause reached the generated file verbatim: the harness
/// called a function named `old`, which exists nowhere, and the whole check
/// died with the compiler's "cannot find function `old` in this scope"
/// dressed up as an internal tool error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryValue {
    /// The generated binding's name (`__ply_old_0`).
    pub ident: String,
    /// The expression to read into it, as source text.
    pub expr: String,
}

/// Rewrites every `old(expr)` in `body` into a plain reference to a binding
/// the caller must emit **before** the call, and returns those bindings in
/// evaluation order. A clause with no `old()` comes back unchanged and with
/// an empty list, so every caller can run this unconditionally.
pub(crate) fn lift_entry_values(body: &Expr) -> (Expr, Vec<EntryValue>) {
    use syn::visit_mut::VisitMut;

    struct Lifter {
        found: Vec<EntryValue>,
    }
    impl VisitMut for Lifter {
        fn visit_expr_mut(&mut self, e: &mut Expr) {
            // Depth first, so the bindings come out in the order a reader
            // of the clause meets them, left to right.
            syn::visit_mut::visit_expr_mut(self, e);
            let Expr::Call(call) = &*e else { return };
            let Expr::Path(path) = call.func.as_ref() else {
                return;
            };
            if !path.path.is_ident("old") || call.args.len() != 1 {
                return;
            }
            let ident = format!("__ply_old_{}", self.found.len());
            self.found.push(EntryValue {
                ident: ident.clone(),
                expr: call.args[0].to_token_stream().to_string(),
            });
            *e = syn::parse_str::<Expr>(&ident).expect("an identifier is an expression");
        }
    }

    let mut rewritten = body.clone();
    let mut lifter = Lifter { found: Vec::new() };
    lifter.visit_expr_mut(&mut rewritten);
    (rewritten, lifter.found)
}

/// Rewrites every bare `self` in `body` to `__ply_receiver` -- the name a
/// generated harness already binds the receiver Ply built under
/// (`fuzz_gen::receiver_preamble`). A method's postcondition is spliced into
/// the generated test as a free-standing expression outside any `impl`
/// block, where the literal keyword `self` means nothing
/// (`error[E0424]: expected value, found module `self``) -- the most
/// natural thing a method's own promise can say (relating its result to the
/// receiver it was called on) used to make that promise uncheckable. Called
/// only when the checked function actually has a receiver
/// (`ContractFn::receiver.is_some()`); every other clause comes back
/// unchanged, so a caller can also run this unconditionally without
/// consequence. This rewrite happens *before* `old()` is lifted, so
/// `old(self.a)` reads the receiver's value on entry the same way
/// `old(param)` does.
pub(crate) fn rewrite_self_to_receiver(body: &Expr) -> Expr {
    use syn::visit_mut::VisitMut;

    struct SelfRewriter;
    impl VisitMut for SelfRewriter {
        fn visit_expr_mut(&mut self, e: &mut Expr) {
            syn::visit_mut::visit_expr_mut(self, e);
            if let Expr::Path(p) = e
                && p.path.is_ident("self")
            {
                *e = syn::parse_str::<Expr>("__ply_receiver")
                    .expect("an identifier is an expression");
            }
        }
    }

    let mut rewritten = body.clone();
    SelfRewriter.visit_expr_mut(&mut rewritten);
    rewritten
}

/// The `let` statements that read the entry values, one per binding, each
/// followed by a newline and `indent` so the caller can splice them in at
/// the point the call is about to be written. `.clone()` rather than a bare
/// move: it is a no-op for the scalars this reaches in practice and keeps a
/// collection parameter usable in the call that follows.
pub(crate) fn entry_value_lets(values: &[EntryValue], indent: &str) -> String {
    let mut out = String::new();
    for v in values {
        out.push_str(&format!(
            "let {ident} = ({expr}).clone();\n{indent}",
            ident = v.ident,
            expr = v.expr
        ));
    }
    out
}

/// Splits a top-level `||` chain into its arms, left to right, the same
/// order `||` itself reads in (2026-09-02, the branch-decided measurement:
/// "record which branch of the promise actually decided each case"). `a ||
/// b || c` parses as `(a || b) || c` (`||` is left-associative), so this
/// walks down the left spine and collects each right-hand side as it goes,
/// producing `[a, b, c]`.
///
/// Returns `None` for any shape that is not a bare `||` at the top --
/// including a body with no `||` at all, and one buried under `&&` or some
/// other operator -- rather than a shape it was not asked to read. A single
/// layer of parentheses around the whole body is stripped first, since
/// `(a || b)` is the same promise as `a || b`; parentheses *inside* an arm
/// are left to `widen` to see through, unrelated to whether the arm itself
/// is `||`-shaped.
pub fn flatten_top_level_or(expr: &Expr) -> Option<Vec<Expr>> {
    fn strip_paren(mut e: &Expr) -> &Expr {
        while let Expr::Paren(p) = e {
            e = &p.expr;
        }
        e
    }
    fn flatten_into(expr: &Expr, out: &mut Vec<Expr>) {
        let stripped = strip_paren(expr);
        if let Expr::Binary(bin) = stripped
            && matches!(bin.op, BinOp::Or(_))
        {
            flatten_into(&bin.left, out);
            out.push((*bin.right).clone());
        } else {
            out.push(stripped.clone());
        }
    }

    let top = strip_paren(expr);
    let Expr::Binary(bin) = top else { return None };
    if !matches!(bin.op, BinOp::Or(_)) {
        return None;
    }
    let mut out = Vec::new();
    flatten_into(top, &mut out);
    Some(out)
}

/// [`flatten_top_level_or`], rendered as the newbie-bar text a diagnostic
/// quotes back at the reader -- each arm's source, tidied the same way
/// [`harness::tidy_contract_text`] already cleans up a whole contract's
/// text for a diagnostic or a generated test's doc comment. Kept in this
/// crate (rather than `ply-cli`, the only caller) so the caller building the
/// branch-decided disclosure never needs `syn`/`quote` as dependencies of
/// its own just to read an `Expr` back out as a string.
pub fn or_arm_texts(body: &Expr) -> Option<Vec<String>> {
    let arms = flatten_top_level_or(body)?;
    Some(
        arms.iter()
            .map(|a| crate::harness::tidy_contract_text(&a.to_token_stream().to_string()))
            .collect(),
    )
}

/// Recursively widens every arithmetic/comparison subexpression to `i128` so
/// the rendered assertion can never itself overflow while checking the
/// contract (the D7 plan's "spike trap" fix: `result == x + 1` at x = 255
/// must not fail with "attempt to add with overflow" instead of stating the
/// broken contract). Leaves non-arithmetic constructs (deref, field access,
/// method calls used as opaque leaves, logical operators) structurally
/// alone -- only their *scalar* leaves get cast to i128, and (2026-09-01)
/// only when both sides of a comparison are [`is_provably_numeric`]: casting
/// a leaf that is not a number at all -- text, an `Option`, a struct or enum
/// -- is not a widening, it is a compile error (`error[E0605]:
/// non-primitive cast`/`error[E0606]: casting &str as i128 is invalid`),
/// and because every check in a crate shares one generated harness (§5.4c),
/// one such comparison used to turn every *other* function's evidence into
/// a tool error too. `cf` supplies the only two things this decision can be
/// made from -- the checked fn's parameter types and its return type (see
/// [`is_provably_numeric`]'s own doc for the exact rule).
pub(crate) fn widen(expr: &Expr, cf: &ContractFn) -> proc_macro2::TokenStream {
    match expr {
        Expr::Binary(bin) => {
            let op = bin.op;
            match op {
                BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_) | BinOp::Rem(_) => {
                    let l = widen_leaf(&bin.left, cf);
                    let r = widen_leaf(&bin.right, cf);
                    quote::quote!((#l #op #r))
                }
                BinOp::Eq(_)
                | BinOp::Ne(_)
                | BinOp::Lt(_)
                | BinOp::Le(_)
                | BinOp::Gt(_)
                | BinOp::Ge(_) => {
                    if is_provably_numeric(&bin.left, cf) && is_provably_numeric(&bin.right, cf) {
                        let l = widen_leaf(&bin.left, cf);
                        let r = widen_leaf(&bin.right, cf);
                        quote::quote!((#l) #op (#r))
                    } else {
                        // Not provably numeric on both sides: emit the
                        // comparison exactly as the user wrote it. This can
                        // never itself break compilation (it is legal Rust
                        // already, or `rustc` would have refused the
                        // function before Ply ever saw it) -- only casting
                        // it could.
                        expr.to_token_stream()
                    }
                }
                BinOp::And(_) | BinOp::Or(_) => {
                    let l = widen(&bin.left, cf);
                    let r = widen(&bin.right, cf);
                    quote::quote!((#l) #op (#r))
                }
                _ => expr.to_token_stream(),
            }
        }
        Expr::Paren(p) => widen(&p.expr, cf),
        _ => expr.to_token_stream(),
    }
}

/// Widens one leaf of an arithmetic/comparison expression: recurses through
/// nested arithmetic (so `a + b * c` promotes every operand, not just the
/// outermost), and casts anything else (literal, path, deref, method call,
/// field access, existing cast) to `i128` at the leaf.
///
/// A comparison or logical operator (`==`, `&&`, ...) reaching here is one
/// nested *inside* another comparison as a leaf (`*result == (a == b)`) --
/// `widen`'s own top-level match only descends into `&&`/`||` when one of
/// them is the *outermost* operator, so a comparison used as a leaf used to
/// fall through to the catch-all below, which cast its token stream to
/// `i128` with no parens of its own: `a == b` became `a == b as i128`, and
/// because `as` binds tighter than `==`, that parses as `a == (b as i128)`
/// -- comparing the wrong types (`error[E0308]`) instead of casting the
/// whole comparison. Recursing through `widen` here (rather than taking the
/// expression's tokens verbatim) also keeps any arithmetic on either side of
/// that nested comparison itself widened to `i128`, so a mixed case (`a + 1
/// == b`, nested as a leaf) still cannot overflow while being checked --
/// only the outer parenthesise-then-cast is new, not a second, weaker path
/// for arithmetic. This cast is always safe regardless of what the nested
/// comparison itself compares: `widen` on that inner expression already
/// applies its own [`is_provably_numeric`] gate to *its* operands, and
/// whatever it evaluates to is a plain `bool` -- always castable to `i128`.
///
/// This function itself is reached from `widen`'s comparison arm only
/// *after* that gate has confirmed both sides numeric, so every leaf it
/// sees here is safe to cast; the one exception is the arithmetic arm
/// immediately below, entered unconditionally for `+`/`-`/`*`/`/`/`%`
/// (arithmetic operators are never applied to a non-numeric leaf without
/// `rustc` refusing the function outright, before Ply ever sees it, so no
/// gate is needed there).
fn widen_leaf(expr: &Expr, cf: &ContractFn) -> proc_macro2::TokenStream {
    match expr {
        Expr::Binary(bin)
            if matches!(
                bin.op,
                BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_) | BinOp::Rem(_)
            ) =>
        {
            widen(expr, cf)
        }
        Expr::Binary(bin)
            if matches!(
                bin.op,
                BinOp::Eq(_)
                    | BinOp::Ne(_)
                    | BinOp::Lt(_)
                    | BinOp::Le(_)
                    | BinOp::Gt(_)
                    | BinOp::Ge(_)
                    | BinOp::And(_)
                    | BinOp::Or(_)
            ) =>
        {
            let inner = widen(expr, cf);
            quote::quote!(((#inner) as i128))
        }
        Expr::Paren(p) => widen_leaf(&p.expr, cf),
        other => quote::quote!((#other as i128)),
    }
}

/// The plain integer scalars widen may safely cast to `i128`, plus
/// `bool`/`char`/`f32`/`f64` -- every `RustType` shape Rust's own `as`
/// operator can cast to `i128` without a compile error (verified
/// directly against `rustc`, not assumed: a bare fieldless enum can *also*
/// take this cast, but only until it gains a `Drop` impl, so enums are
/// deliberately not on this list -- see the `tests` module's own
/// `an_enum_variant_comparison_is_rendered_verbatim_never_cast_to_i128`).
/// Every other `RustType` shape -- `Option`,
/// `Result`, `Vec`/`VecU8`/`BTreeSet`/`Array`, `String`, `NonZero`,
/// `Duration`, a struct or enum, `SelfType`, `Unit`, `Unsupported` -- is a
/// container or opaque wrapper `as i128` cannot reach through, and answers
/// `false`.
fn is_numeric_rust_type(ty: &RustType) -> bool {
    matches!(
        ty,
        RustType::U8
            | RustType::U16
            | RustType::U32
            | RustType::U64
            | RustType::I8
            | RustType::I16
            | RustType::I32
            | RustType::I64
            | RustType::Usize
            | RustType::Isize
            | RustType::Bool
            | RustType::Char
            | RustType::F32
            | RustType::F64
    )
}

/// Whether a `syn::Type` written as an explicit cast target (`x as <ty>`) is
/// itself one of Rust's plain integer primitives -- decided directly from
/// the cast's own spelling, not through `RustType`'s vocabulary (which has
/// no `i128`/`u128` variant of its own, and would wrongly answer "not
/// numeric" for a cast that is already exactly the width widen casts to).
fn is_numeric_cast_target(ty: &syn::Type) -> bool {
    let syn::Type::Path(tp) = ty else {
        return false;
    };
    let Some(seg) = tp.path.segments.last() else {
        return false;
    };
    matches!(
        seg.ident.to_string().as_str(),
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
    )
}

/// The identifier a checked fn's own `#[ply::ensures(|result| ...)]` closure
/// binds its result to (conventionally `result`, but not enforced) -- the
/// one bare name in the contract that resolves to the *return* type rather
/// than a parameter's.
fn cf_result_ident(cf: &ContractFn) -> Option<String> {
    let (closure, _) = cf.ensures.as_ref()?;
    closure_result_ident(closure).ok()
}

/// A bare identifier's resolved type, decided only from what a `ContractFn`
/// already knows about itself: a parameter's declared type, or -- if the
/// name is the closure's own result binding -- the function's return type.
/// `false` for any name that resolves to neither (a local Ply cannot see
/// the type of, e.g. a constant or a binding introduced elsewhere).
fn resolved_type_is_numeric(name: &str, cf: &ContractFn) -> bool {
    if let Some(p) = cf.params.iter().find(|p| p.name == name) {
        return is_numeric_rust_type(&p.ty);
    }
    if cf_result_ident(cf).as_deref() == Some(name) {
        return is_numeric_rust_type(&cf.return_type);
    }
    false
}

/// Is `expr` **provably** a number? Decided only from information already
/// available -- the checked fn's own parameter and return types -- never
/// guessed:
///
/// - a numeric literal is numeric;
/// - a name that refers to a parameter (or the result) whose resolved type
///   is a number is numeric;
/// - a dereference or parenthesised form of a numeric thing is numeric;
/// - an explicit cast to a numeric type is numeric;
/// - arithmetic (`+`/`-`/`*`/`/`/`%`) whose operands are all numeric is
///   numeric;
/// - a comparison or logical expression (`==`, `&&`, ...) is always
///   numeric here, whatever it compares: it always evaluates to `bool`,
///   which is always castable to `i128` (see [`is_numeric_rust_type`]) --
///   `widen_leaf`'s own nested-comparison arm applies this same
///   [`is_provably_numeric`] gate to *that* expression's own operands
///   before it casts anything, so nothing about its insides is assumed;
/// - anything else -- a method call, a field access, a path to a constant,
///   an enum variant -- is **not**, because there is no information here
///   that could prove otherwise.
///
/// Conservative by design: treating a genuinely numeric expression as "not
/// numeric" only means widen leaves that one comparison uncast, which is
/// always legal Rust (`rustc` already accepted the function's own body with
/// that comparison in it, unwidened, before Ply ever ran); treating a
/// non-numeric expression as numeric is what casts a `&str`/`Option`/
/// struct/enum `as i128` and breaks compilation for the whole crate's
/// shared generated harness. When unsure, this answers `false`.
fn is_provably_numeric(expr: &Expr, cf: &ContractFn) -> bool {
    match expr {
        Expr::Lit(lit) => matches!(lit.lit, syn::Lit::Int(_)),
        Expr::Path(p) => match p.path.get_ident() {
            Some(ident) => resolved_type_is_numeric(&ident.to_string(), cf),
            None => false,
        },
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => is_provably_numeric(&u.expr, cf),
        Expr::Paren(p) => is_provably_numeric(&p.expr, cf),
        Expr::Cast(c) => is_numeric_cast_target(&c.ty),
        Expr::Binary(bin)
            if matches!(
                bin.op,
                BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_) | BinOp::Rem(_)
            ) =>
        {
            is_provably_numeric(&bin.left, cf) && is_provably_numeric(&bin.right, cf)
        }
        Expr::Binary(bin)
            if matches!(
                bin.op,
                BinOp::Eq(_)
                    | BinOp::Ne(_)
                    | BinOp::Lt(_)
                    | BinOp::Le(_)
                    | BinOp::Gt(_)
                    | BinOp::Ge(_)
                    | BinOp::And(_)
                    | BinOp::Or(_)
            ) =>
        {
            true
        }
        _ => false,
    }
}

/// Every `use`-imported path a contract's own text names, as raw segments.
///
/// A promise may name something the function's file imported rather than
/// defined (`Ordering`, from a `use std::cmp::Ordering;` at the top of the
/// module). Both places that splice contract text -- the sampling harness
/// and the counterexample replay test -- need those names in scope, and a
/// second copy of this rule is how the two came to disagree once already
/// (2026-09-04). `self::`/`super::` are skipped: they resolve against the
/// generated module rather than the function's, so importing them would
/// bring in the wrong thing rather than nothing.
pub fn contract_use_paths(cf: &ContractFn) -> Vec<Vec<String>> {
    let mut idents = std::collections::BTreeSet::new();
    if let Some((expr, _)) = &cf.requires {
        crate::fuzz_gen::collect_leading_idents(expr, &mut idents);
    }
    if let Some((closure, _)) = &cf.ensures {
        crate::fuzz_gen::collect_leading_idents(&closure.body, &mut idents);
    }
    idents
        .into_iter()
        .filter_map(|ident| cf.use_aliases.get(&ident))
        .filter(|segments| {
            !matches!(
                segments.first().map(String::as_str),
                Some("self") | Some("super")
            )
        })
        .cloned()
        .collect()
}

fn scalar_literal(v: &WitnessValue, ty: &RustType) -> Result<String> {
    let ty_name = ty.scalar_rust_name().unwrap_or("i128");
    Ok(match v {
        WitnessValue::UInt(u) => format!("{u}{ty_name}"),
        WitnessValue::Int(i) => format!("{i}{ty_name}"),
        WitnessValue::Bool(b) => format!("{b}"),
        WitnessValue::VecU8(_) => bail!("scalar_literal called on a Vec<u8> witness value"),
        WitnessValue::Duration(..) => {
            bail!(
                "scalar_literal called on a Duration witness value -- render_cex_test has its own arm for it"
            )
        }
        WitnessValue::Str(_) => {
            bail!(
                "scalar_literal called on a String witness value -- render_cex_test has its own arm for it"
            )
        }
    })
}

fn vec_literal(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(|b| format!("{b}u8")).collect();
    format!("vec![{}]", items.join(", "))
}

pub struct RenderedTest {
    pub test_name: String,
    pub source: String,
}

/// Wraps one or more rendered tests into the single generated-file source
/// `harness::write_generated_test` writes in-crate (D7's in-crate placement,
/// same mechanism as the proof module) -- `use super::*;` brings the target
/// function (and any private items it needs) into scope for every test.
pub fn wrap_test_module(tests: &[RenderedTest]) -> String {
    let mut out = String::from(
        "//! Generated by Ply -- do not edit. D7 rendered contract-assertion tests.\n#[cfg(test)]\nuse super::*;\n\n",
    );
    for t in tests {
        out.push_str(&t.source);
        out.push('\n');
    }
    out
}

/// Renders one `ply_cex_<fn>_<NN>` test asserting `cf`'s ensures clause
/// against the witness `values` (already decoded, in parameter order).
/// `check_label` is the check that found the violation (e.g. `bounded(2)`),
/// embedded in the doc comment. `index` numbers the test when a function has
/// more than one witness.
pub fn render_cex_test(
    cf: &ContractFn,
    values: &[WitnessValue],
    check_label: &str,
    diagnostic_code: &str,
    index: u32,
) -> Result<RenderedTest> {
    let Some((closure, contract_text)) = &cf.ensures else {
        bail!("cannot render a cex test for a function with no #[ply::ensures]");
    };
    if values.len() != cf.params.len() {
        bail!(
            "witness has {} values but `{}` has {} parameters",
            values.len(),
            cf.name,
            cf.params.len()
        );
    }

    let mut lets = String::new();
    let mut call_args = Vec::new();
    for (p, v) in cf.params.iter().zip(values.iter()) {
        match (&p.ty, v) {
            (RustType::VecU8, WitnessValue::VecU8(bytes)) => {
                lets.push_str(&format!(
                    "    let {name}: Vec<u8> = {lit};\n",
                    name = p.name,
                    lit = vec_literal(bytes)
                ));
            }
            (RustType::Duration, WitnessValue::Duration(secs, nanos)) => {
                lets.push_str(&format!(
                    "    let {name}: std::time::Duration = std::time::Duration::new({secs}u64, {nanos}u32);\n",
                    name = p.name,
                ));
            }
            (RustType::String, WitnessValue::Str(text)) => {
                // `str`'s own `Debug` *is* a Rust string literal -- quotes,
                // backslashes and control characters all escaped -- so there
                // is nothing here for Ply to get wrong by hand.
                lets.push_str(&format!(
                    "    let {name}: String = {lit:?}.to_string();\n",
                    name = p.name,
                    lit = text,
                ));
            }
            (RustType::NonZero(inner), val) => {
                let inner_lit = scalar_literal(val, inner)?;
                let suffix = inner
                    .nonzero_suffix()
                    .expect("a NonZero's inner is always a valid nonzero integer");
                lets.push_str(&format!(
                    "    let {name}: std::num::NonZero{suffix} = std::num::NonZero{suffix}::new({inner_lit}).unwrap();\n",
                    name = p.name,
                ));
            }
            (other, val) => {
                let lit = scalar_literal(val, other)?;
                let ty_name = other.scalar_rust_name().unwrap_or("i128");
                lets.push_str(&format!(
                    "    let {name}: {ty} = {lit};\n",
                    name = p.name,
                    ty = ty_name,
                    lit = lit
                ));
            }
        }
        call_args.push(if p.by_ref {
            format!("&{}", p.name)
        } else {
            p.name.clone()
        });
    }

    let result_ident = closure_result_ident(closure)?;
    let _ = result_ident; // Kani's own closure names it `result`; we bind the same name below.

    // `old(expr)` is read into its own binding first: the value on entry is
    // only the value on entry if it is read before the call.
    let (checked_body, entry_values) = lift_entry_values(&closure.body);
    let entry_lets = entry_value_lets(&entry_values, "    ");

    let widened = widen(&checked_body, cf);
    let widened_str = widened.to_string();

    let test_name = format!("ply_cex_{}_{:02}", cf.ident(), index);

    let message = render_message(cf, &checked_body, contract_text, diagnostic_code)?;

    // A promise may name a helper that lives beside the function it is
    // written on, and the contract text is spliced in exactly as written.
    // The fuzz harness resolves those because it imports the function's own
    // module; this test sits at the crate root under `use super::*`, where a
    // name from a nested module is not in scope -- so the test Ply writes
    // into the user's crate would not compile, and `cargo test` would break
    // for a reason they did not cause (2026-09-04, found on a claim whose
    // promise calls its own function).
    // `import_path()` already knows how many segments to drop -- one for a
    // free function, two for `Type::method`, because the second-to-last
    // segment there is a type and `use crate::Bucket::*` does not compile.
    // Reusing it is what keeps this from being a second, disagreeing copy
    // of the same split (2026-09-04 review, which caught exactly that).
    let module_import = match cf.import_path().rsplit_once("::") {
        Some((module, _)) if cf.is_method => {
            format!("    #[allow(unused_imports)]\n    use crate::{module}::*;\n")
        }
        _ if cf.is_method => String::new(),
        // The allow is not optional: most promises name no sibling at all,
        // so without it every nested replay carries an unused-import
        // warning -- and a warning in a file the user never wrote is a
        // broken build in any crate that denies them.
        _ => match cf.path.rsplit_once("::") {
            Some((module, _)) => {
                format!("    #[allow(unused_imports)]\n    use crate::{module}::*;\n")
            }
            None => String::new(),
        },
    };

    // The same names the sampling harness brings in, spelled for a file
    // that lives inside the crate rather than beside it.
    let contract_imports: String = contract_use_paths(cf)
        .into_iter()
        .map(|segments| {
            format!(
                "    #[allow(unused_imports)]\n    use {};\n",
                segments.join("::")
            )
        })
        .collect();

    let source = format!(
        "// Reproduces the counterexample for `{fname}` found by check\n\
         // `{check_label}` (diagnostic {code}). This test fails until the\n\
         // function body or its contract change, and passes once they agree.\n\
         #[cfg(test)]\n\
         #[test]\n\
         #[allow(non_snake_case)]\n\
         fn {test_name}() {{\n\
         {module_import}\
         {contract_imports}\
         {lets}\n\
         \x20\x20\x20\x20{entry_lets}let result = &{fname}({args});\n\n\
         \x20\x20\x20\x20// Contract under test: #[ply::ensures({contract_text})]\n\
         \x20\x20\x20\x20// Arithmetic is evaluated in i128 so the test can report the broken\n\
         \x20\x20\x20\x20// promise instead of overflowing while checking it.\n\
         \x20\x20\x20\x20let __ply_ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {widened_expr}));\n\
         \x20\x20\x20\x20match __ply_ok {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20Ok(true) => {{}}\n\
         \x20\x20\x20\x20\x20\x20\x20\x20Ok(false) => panic!({message}),\n\
         \x20\x20\x20\x20\x20\x20\x20\x20Err(_) => panic!(\"the contract's own check crashed at this input: the contract or a `pure` helper it calls is wrong for this input, which itself means the contract could not even be evaluated here. ({code})\"),\n\
         \x20\x20\x20\x20}}\n\
         }}\n",
        fname = cf.path,
        check_label = check_label,
        code = diagnostic_code,
        test_name = test_name,
        module_import = module_import,
        contract_imports = contract_imports,
        lets = lets,
        entry_lets = entry_lets,
        args = call_args.join(", "),
        contract_text = contract_text,
        widened_expr = widened_str,
        message = message,
    );

    Ok(RenderedTest { test_name, source })
}

fn closure_result_ident(closure: &ExprClosure) -> Result<String> {
    let Some(first) = closure.inputs.first() else {
        bail!("ensures closure has no parameter");
    };
    match first {
        syn::Pat::Ident(pi) => Ok(pi.ident.to_string()),
        _ => bail!("ensures closure parameter must be a plain identifier"),
    }
}

/// Builds the `panic!(...)` argument list (format string + interpolated
/// args) for a top-level comparison. Falls back to a generic
/// "this expression evaluated to false" message for anything else, per the
/// D7 plan's own fallback clause -- never a bare, uninterpreted panic. Also
/// falls back to that same generic message when the comparison's sides are
/// not [`is_provably_numeric`] (2026-09-01): the value-naming message below
/// only works by casting each side to `i128` so it can share one format
/// string regardless of the comparison's real type, and that cast is
/// exactly what breaks compilation for a non-numeric comparison -- so this
/// message must decline the same cases `widen` does, for the same reason.
fn render_message(cf: &ContractFn, body: &Expr, contract_text: &str, code: &str) -> Result<String> {
    let fname = &cf.path;
    if let Expr::Binary(bin) = body
        && matches!(
            bin.op,
            BinOp::Eq(_) | BinOp::Ne(_) | BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_)
        )
        && is_provably_numeric(&bin.left, cf)
        && is_provably_numeric(&bin.right, cf)
    {
        let l = widen_leaf(&bin.left, cf).to_string();
        let r = widen_leaf(&bin.right, cf).to_string();
        let msg = format!(
            "\"Broken promise in `{fname}`: the function declares the postcondition \\\n         `{contract_text}` -- a postcondition is the guarantee a function makes about \\\n         its return value. For this input, the left side of the contract evaluated to \\\n         {{}}, and the right side evaluated to {{}}, which does not satisfy the contract's \\\n         comparison. One of the two is wrong: fix the body or fix the `#[ply::ensures]` \\\n         line, and this test will pass. ({code})\", {l}, {r}"
        );
        return Ok(msg);
    }
    Ok(format!(
        "\"Broken promise in `{fname}`: the function declares the postcondition `{contract_text}` \\\n         -- a postcondition is the guarantee a function makes about its return value. For \\\n         this input, that expression evaluated to false. Fix the body or fix the \\\n         `#[ply::ensures]` line, and this test will pass. ({code})\""
    ))
}

// -- 2026-09-02: the branch-decided measurement (CLAUDE.md, "record which
// branch of the promise actually decided each case") needs to read a
// top-level `||` chain back out of the AST honestly -- these pin exactly
// what `flatten_top_level_or` returns before any codegen is built on top
// of it.
#[cfg(test)]
mod flatten_top_level_or_tests {
    use super::*;
    use quote::ToTokens;

    fn arm_texts(expr: &Expr) -> Option<Vec<String>> {
        flatten_top_level_or(expr).map(|arms| {
            arms.iter()
                .map(|a| a.to_token_stream().to_string())
                .collect()
        })
    }

    #[test]
    fn a_two_arm_or_chain_splits_left_to_right() {
        let expr: Expr = syn::parse_str("a || b").unwrap();
        let texts = arm_texts(&expr).expect("a bare `||` must split");
        assert_eq!(texts, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn a_three_arm_or_chain_splits_in_source_order() {
        // `a || b || c` parses as `(a || b) || c` (`||` is left-
        // associative) -- this must still read out as `[a, b, c]`, the
        // order a person reads the line in, not `[(a || b), c]`.
        let expr: Expr = syn::parse_str("a || b || c").unwrap();
        let texts = arm_texts(&expr).expect("a three-arm `||` chain must split");
        assert_eq!(
            texts,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn a_body_with_no_top_level_or_returns_none() {
        let expr: Expr = syn::parse_str("*result == x").unwrap();
        assert!(
            flatten_top_level_or(&expr).is_none(),
            "a shape with no top-level `||` must get no split, never an invented one"
        );
    }

    #[test]
    fn or_nested_under_and_is_not_a_top_level_or() {
        // `a && (b || c)` -- the `||` is real, but it is not the shape this
        // whole promise is at the top, and CLAUDE.md is explicit: refuse
        // quietly rather than guess for a shape not asked about.
        let expr: Expr = syn::parse_str("a && (b || c)").unwrap();
        assert!(
            flatten_top_level_or(&expr).is_none(),
            "an `||` buried under `&&` is not a top-level `||` chain"
        );
    }

    #[test]
    fn one_layer_of_parens_around_the_whole_body_is_stripped() {
        let expr: Expr = syn::parse_str("(a || b)").unwrap();
        let texts = arm_texts(&expr).expect("a parenthesised `||` is still a `||`");
        assert_eq!(texts, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn or_arm_texts_renders_each_arm_the_way_a_reader_wrote_it() {
        let expr: Expr = syn::parse_str("x < 100 || result . unwrap () == x").unwrap();
        let texts = or_arm_texts(&expr).expect("this is a bare `||` chain");
        assert_eq!(
            texts,
            vec!["x < 100".to_string(), "result.unwrap() == x".to_string()],
            "each arm must be tidied the same way a whole contract's text already is, not \
             left with `quote`'s own token-by-token spacing"
        );
    }

    #[test]
    fn a_real_ensures_style_or_chain_splits_into_its_two_conditions() {
        // The exact shape from the real defect this measurement exists for
        // (CLAUDE.md): `semver`'s `Version::parse` promise, `!text.contains(
        // ' ') || result.is_err()`.
        let expr: Expr = syn::parse_str("! text . contains(' ') || result . is_err()").unwrap();
        let texts = arm_texts(&expr).expect("this real-world shape must split");
        assert_eq!(texts.len(), 2);
        assert!(texts[0].contains("contains"));
        assert!(texts[1].contains("is_err"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::kani::WitnessValue;
    use crate::harness::discover_fn;

    fn discover(src: &str, name: &str) -> ContractFn {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, src).unwrap();
        discover_fn(&path, name).unwrap()
    }

    /// Seventeen of the twenty-three promises in Ply's own library take
    /// text, so "we cannot write a string back out as Rust source" made the
    /// counterexample unreplayable in the common case, not an edge one.
    /// Found 2026-09-04 by breaking `schema::dotted` on purpose: the report
    /// named the failing input and then said it could not turn it into a
    /// test.
    #[test]
    fn a_string_counterexample_renders_as_a_runnable_test() {
        let cf = discover(
            r#"
#[ply::ensures(|result| result.len() > 0)]
pub fn head(s: &str) -> String { s.chars().take(1).collect() }
"#,
            "head",
        );
        let rendered = render_cex_test(
            &cf,
            &[WitnessValue::Str(String::new())],
            "fuzz(256)",
            "P0502",
            1,
        )
        .unwrap();
        assert!(
            rendered
                .source
                .contains("let s: String = \"\".to_string();"),
            "the failing string has to be written back out as a Rust literal:\n{}",
            rendered.source
        );
        assert!(
            rendered.source.contains("head(&s)"),
            "a `&str` parameter is passed by reference, so the binding is borrowed:\n{}",
            rendered.source
        );
    }

    /// A witness Ply cannot escape correctly is worse than one it refuses:
    /// the replay test would fail to compile inside the user's own crate.
    #[test]
    fn a_string_counterexample_with_quotes_and_newlines_stays_a_valid_literal() {
        let cf = discover(
            r#"
#[ply::ensures(|result| result.len() > 0)]
pub fn head(s: &str) -> String { s.chars().take(1).collect() }
"#,
            "head",
        );
        let rendered = render_cex_test(
            &cf,
            &[WitnessValue::Str("a\"b\\c\nd".to_string())],
            "fuzz(256)",
            "P0502",
            1,
        )
        .unwrap();
        assert!(
            rendered
                .source
                .contains(r#"let s: String = "a\"b\\c\nd".to_string();"#),
            "quotes, backslashes and newlines must be escaped for Rust:\n{}",
            rendered.source
        );
    }

    /// The worst failure this file can produce: a test written into the
    /// user's own crate that does not compile, so `cargo test` breaks for a
    /// reason they did not cause.
    ///
    /// This one builds a real crate and runs it, because every cheaper
    /// version of it was wrong. Two string-contains tests stood here first
    /// and passed while the renderer emitted `use crate::Bucket::*;` for an
    /// associated function -- a hard compile error -- and an unused import
    /// for every promise that names no sibling, which is an error under
    /// `-D warnings`. Asserting on the text is how both got through; the
    /// only assertion that distinguishes them is whether rustc accepts it.
    #[test]
    fn every_rendered_shape_compiles_and_fails_on_its_promise_not_its_syntax() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();

        // Three shapes, each rendered from source that really declares the
        // promise -- a free function at the crate root, one in a nested
        // module whose promise calls a sibling, and a receiverless
        // associated function (`Type::method`, where the module split has
        // one more segment to drop).
        let src = r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 { x.min(100) + 1 }

pub mod helpers {
    #[ply::ensures(|result| *result == twice(x))]
    pub fn double(x: u32) -> u32 { x * 3 }
    pub fn twice(x: u32) -> u32 { x + x }

    // A nested claim whose promise names no sibling -- the common case,
    // and the one that showed the import has to carry its own `allow`.
    #[ply::ensures(|result| *result > x)]
    pub fn shrink(x: u32) -> u32 { x }

    // A promise naming something the module imported rather than defined.
    // The sampling harness resolves this from the file's own `use` items;
    // the replay test has to as well, or the two disagree about what a
    // promise is even allowed to say.
    use std::cmp::Ordering;
    #[ply::ensures(|result| *result == Ordering::Less)]
    pub fn compare(x: u32) -> Ordering { x.cmp(&3) }
}

pub struct Bucket { pub n: u32 }
impl Bucket {
    #[ply::ensures(|result| result.n == 0)]
    pub fn new(n: u32) -> Bucket { Bucket { n } }
}
"#;
        let lib = root.join("src/lib.rs");
        std::fs::write(&lib, src).unwrap();

        let mut tests = Vec::new();
        for (i, name) in [
            "clamp",
            "helpers::double",
            "helpers::shrink",
            "helpers::compare",
            "Bucket::new",
        ]
        .iter()
        .enumerate()
        {
            let cf = discover_fn(&lib, name).unwrap();
            tests.push(
                render_cex_test(
                    &cf,
                    &[WitnessValue::UInt(3)],
                    "fuzz(8)",
                    "P0502",
                    i as u32 + 1,
                )
                .unwrap(),
            );
        }

        // The crate that gets built is the same source with the attributes
        // stripped, so this needs no proc-macro dependency -- the shapes
        // and paths are what is under test, not the attribute.
        let plain: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("#[ply::"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(
            &lib,
            format!("{plain}\n\n// Ply-generated\nmod ply_generated_cex;\n"),
        )
        .unwrap();
        std::fs::write(
            root.join("src/ply_generated_cex.rs"),
            wrap_test_module(&tests),
        )
        .unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"plycex\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .unwrap();

        let out = std::process::Command::new(env!("CARGO"))
            .args(["test", "--lib"])
            // A warning in a file the user never wrote is a broken build in
            // any crate that denies them, so it is a failure here too.
            .env("RUSTFLAGS", "-D warnings")
            .current_dir(root)
            .output()
            .expect("cargo test");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        assert!(
            !combined.contains("could not compile"),
            "the tests Ply writes into a user's crate must build:\n{combined}"
        );
        assert_eq!(
            combined.matches("Broken promise").count(),
            5,
            "each rendered test must run and fail on its own promise, not be \
             skipped or fail to build:\n{combined}"
        );
    }

    #[test]
    fn renders_clamp_cex_test_fails_for_the_right_reason() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x)]
pub fn clamp(x: u32) -> u32 { x.min(100) }
"#,
            "clamp",
        );
        let values = vec![WitnessValue::UInt(255)];
        let rendered = render_cex_test(&cf, &values, "bounded(2)", "K0502", 1).unwrap();
        assert_eq!(rendered.test_name, "ply_cex_clamp_01");
        assert!(rendered.source.contains("postcondition"));
        assert!(rendered.source.contains("result == x"));
        assert!(rendered.source.contains("255u32"));
    }

    /// The replay test Ply writes when a check finds a failing input has
    /// to compile under plain `cargo test`. A contract that refers to a
    /// parameter's value on entry -- `old(x)` -- rendered that call
    /// verbatim, so the file Ply generated referred to a function that does
    /// not exist and the user's own test run broke on Ply's code.
    #[test]
    fn a_before_value_in_the_contract_is_read_before_the_call_in_the_replay_test() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == old(x) + 1)]
pub fn bump(x: u8) -> u8 { x.saturating_add(1) }
"#,
            "bump",
        );
        let rendered =
            render_cex_test(&cf, &[WitnessValue::UInt(255)], "fuzz(64)", "F0502", 1).unwrap();
        let check_line = rendered
            .source
            .lines()
            .find(|l| l.contains("catch_unwind"))
            .expect("the replay test always evaluates the contract");
        assert!(
            !check_line.replace(' ', "").contains("old("),
            "the line that evaluates the contract must not call a function named `old` -- there \
             is no such function, and the file Ply generates then breaks the user's own \
             `cargo test` run:\n{check_line}"
        );
        assert!(
            check_line.contains("__ply_old_0"),
            "it must read the entry value out of the binding instead:\n{check_line}"
        );
        let snapshot = rendered
            .source
            .find("let __ply_old_0")
            .expect("the entry value must be read into a binding of its own");
        let call = rendered
            .source
            .find("let result = &")
            .expect("the replay test always calls the function");
        assert!(
            snapshot < call,
            "the entry value has to be read before the call:\n{}",
            rendered.source
        );
        assert!(
            rendered.source.contains("old(x) + 1"),
            "the contract is still quoted back to the reader the way they wrote it:\n{}",
            rendered.source
        );
    }

    #[test]
    fn widens_arithmetic_so_overflow_cannot_hide_the_defect() {
        // The spike's own trap: saturating_bump(255) with `result == x + 1`
        // overflows u8 if checked natively. The rendered test must compare
        // in i128, never re-triggering the overflow while checking it.
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == x + 1)]
pub fn saturating_bump(x: u8) -> u8 { x.saturating_add(1) }
"#,
            "saturating_bump",
        );
        let values = vec![WitnessValue::UInt(255)];
        let rendered = render_cex_test(&cf, &values, "bounded(1)", "K0502", 1).unwrap();
        assert!(
            !rendered.source.contains("attempt to add with overflow"),
            "must never bake in the overflow trap's own panic text"
        );
        assert!(
            rendered.source.contains("as i128"),
            "arithmetic must be widened"
        );
        assert!(rendered.source.contains("(x as i128) + (1 as i128)"));
    }

    // -- 2026-08-27: NonZero and Duration witnesses render as the real
    // constructor, never as a bare integer literal a `NonZero`/`Duration`
    // binding could not even accept.

    #[test]
    fn a_nonzero_u32_witness_renders_through_the_public_constructor() {
        let cf = discover(
            r#"
use std::num::NonZeroU32;
#[ply::ensures(|result| *result > 0)]
pub fn get(n: NonZeroU32) -> u32 { n.get() }
"#,
            "get",
        );
        let values = vec![WitnessValue::UInt(0)];
        let rendered = render_cex_test(&cf, &values, "bounded(2)", "K0502", 1).unwrap();
        assert!(
            rendered
                .source
                .contains("std::num::NonZeroU32::new(0u32).unwrap()"),
            "a NonZero witness must render through `NonZero{{X}}::new(..).unwrap()`, never as a \
             bare integer literal a NonZeroU32-typed binding could not accept:\n{}",
            rendered.source
        );
    }

    // -- 2026-08-31: two harness-generation defects found pointing Ply at
    // `semver` (docs/reach-measurement-2.md).

    /// Defect 2: a comparison nested *inside* another comparison as a leaf
    /// (`*result == (a == b)`, a boolean postcondition stated as an
    /// equality of two other equalities) used to render with the cast
    /// binding to the comparison's last operand alone -- `a == b` became
    /// `a == b as i128`, and because `as` binds tighter than `==`, that
    /// parses as `a == (b as i128)`, comparing `u64` to `i128`
    /// (`error[E0308]`) instead of casting the whole comparison.
    #[test]
    fn a_comparison_nested_as_a_leaf_is_parenthesised_before_it_is_cast() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == (a == b))]
pub fn same(a: u64, b: u64) -> bool { a == b }
"#,
            "same",
        );
        let values = vec![WitnessValue::UInt(3), WitnessValue::UInt(4)];
        let rendered = render_cex_test(&cf, &values, "fuzz(64)", "P0502", 1).unwrap();
        let check_line = rendered
            .source
            .lines()
            .find(|l| l.contains("catch_unwind"))
            .expect("the replay test always evaluates the contract");
        assert!(
            !check_line.replace(' ', "").contains("a==basi128"),
            "the nested comparison must not be cast with `as i128` binding to its last operand \
             alone -- that compares the wrong types:\n{check_line}"
        );
        assert!(
            check_line.contains("(a as i128)") && check_line.contains("(b as i128)"),
            "each bare name inside the nested comparison must still be widened to i128 on its \
             own:\n{check_line}"
        );
    }

    /// Defect 1: a method's own postcondition could not mention the
    /// receiver it is called on -- `self` is spliced into the generated
    /// harness as a free-standing expression outside any `impl` block,
    /// where the literal keyword `self` means nothing
    /// (`error[E0424]: expected value, found module `self``).
    /// `rewrite_self_to_receiver` rewrites a bare `self` to the binding a
    /// generated harness already builds the receiver under
    /// (`__ply_receiver`).
    #[test]
    fn rewrite_self_to_receiver_replaces_a_bare_self_with_the_receiver_binding() {
        let expr: Expr = syn::parse_str("*result >= self.a").unwrap();
        let rewritten = rewrite_self_to_receiver(&expr);
        let text = rewritten.to_token_stream().to_string();
        assert!(
            !text.split_whitespace().any(|tok| tok == "self"),
            "no bare `self` may survive the rewrite:\n{text}"
        );
        assert!(
            text.contains("__ply_receiver . a") || text.contains("__ply_receiver.a"),
            "`self.a` must become a read of the receiver binding a generated harness already \
             built:\n{text}"
        );
    }

    /// `self` read alongside a parameter in the same clause -- both must
    /// survive: only the receiver reference is rewritten.
    #[test]
    fn rewrite_self_to_receiver_leaves_other_identifiers_alone() {
        let expr: Expr = syn::parse_str("*result == self.a + extra").unwrap();
        let rewritten = rewrite_self_to_receiver(&expr);
        let text = rewritten.to_token_stream().to_string();
        assert!(
            text.contains("extra"),
            "a parameter read alongside `self` must survive the \
             rewrite untouched:\n{text}"
        );
        assert!(
            text.contains("__ply_receiver . a") || text.contains("__ply_receiver.a"),
            "`self.a` must still become a read of the receiver binding:\n{text}"
        );
    }

    // -- 2026-09-01: widening a comparison's leaves to i128 (so `result ==
    // x + 1` at x's maximum value reports the broken promise instead of
    // overflowing while checking it) used to cast *every* leaf it reached,
    // including ones that are not numbers at all. `&str`, `Option<T>`, and a
    // fieldless enum variant all fail to compile cast `as i128` (E0605/
    // E0606) -- so a promise comparing any of them never got checked at
    // all, and because every check in a crate shares one generated harness,
    // that one comparison broke every other function's evidence too.

    /// The exact shape a `Result`-returning constructor's own postcondition
    /// writes most naturally: comparing the text it was built from back out
    /// through the type. Found pointing Ply at `semver`'s own
    /// `Prerelease::new` (docs/reach-measurement-2.md).
    #[test]
    fn a_text_comparison_is_rendered_verbatim_never_cast_to_i128() {
        let cf = discover(
            r#"
pub struct Wrapper { text: String }
impl Wrapper {
    #[ply::ensures(|result| result.is_err() || result.as_ref().unwrap().as_str() == text)]
    pub fn new(text: &str) -> Result<Wrapper, String> {
        if text.is_empty() { Err("empty".to_string()) } else { Ok(Wrapper { text: text.to_string() }) }
    }
}
"#,
            "Wrapper::new",
        );
        let widened = widen(&cf.ensures.as_ref().unwrap().0.body, &cf).to_string();
        assert!(
            !widened.replace(' ', "").contains("asi128"),
            "a `&str` comparison must never be cast `as i128` -- that is exactly \
             `error[E0606]: casting &str as i128 is invalid`, the defect this test pins:\n{widened}"
        );
        assert!(
            widened.contains("as_str"),
            "the comparison must still be rendered, just not cast:\n{widened}"
        );
    }

    /// An `Option<T>` value compared directly with `==` -- `Option` cannot
    /// be cast `as i128` either (`error[E0605]: non-primitive cast`).
    #[test]
    fn an_option_comparison_is_rendered_verbatim_never_cast_to_i128() {
        let cf = discover(
            r#"
#[ply::ensures(|result| *result == v)]
pub fn identity_opt(v: Option<u32>) -> Option<u32> { v }
"#,
            "identity_opt",
        );
        let widened = widen(&cf.ensures.as_ref().unwrap().0.body, &cf).to_string();
        assert!(
            !widened.replace(' ', "").contains("asi128"),
            "an `Option` comparison must never be cast `as i128`:\n{widened}"
        );
    }

    /// A fieldless enum variant compared directly with `==`. A plain
    /// fieldless enum happens to allow the primitive `as i128` cast Rust
    /// grants "no data, no `Drop`" enums -- so this one carries a (trivial)
    /// `Drop` impl, the ordinary shape that makes that cast a hard compiler
    /// error (`E0320: cannot cast enum ... because it implements Drop`),
    /// confirmed against `rustc` directly rather than assumed.
    #[test]
    fn an_enum_variant_comparison_is_rendered_verbatim_never_cast_to_i128() {
        let cf = discover(
            r#"
#[derive(PartialEq, Eq)]
pub enum Sign { Pos, Neg }
impl Drop for Sign {
    fn drop(&mut self) {}
}
#[ply::ensures(|result| *result == Sign::Pos)]
pub fn always_pos(x: i32) -> Sign { let _ = x; Sign::Pos }
"#,
            "always_pos",
        );
        let widened = widen(&cf.ensures.as_ref().unwrap().0.body, &cf).to_string();
        assert!(
            !widened.replace(' ', "").contains("asi128"),
            "an enum-variant comparison must never be cast `as i128`:\n{widened}"
        );
    }

    #[test]
    fn a_duration_witness_renders_through_duration_new() {
        let cf = discover(
            r#"
use std::time::Duration;
#[ply::ensures(|result| result.subsec_nanos() < 1_000_000_000)]
pub fn identity(d: Duration) -> Duration { d }
"#,
            "identity",
        );
        let values = vec![WitnessValue::Duration(7, 500_000_000)];
        let rendered = render_cex_test(&cf, &values, "bounded(2)", "K0502", 1).unwrap();
        assert!(
            rendered
                .source
                .contains("std::time::Duration::new(7u64, 500000000u32)"),
            "a Duration witness must render through `Duration::new(secs, nanos)`, the same public \
             constructor the harness itself uses to build one:\n{}",
            rendered.source
        );
    }
}
