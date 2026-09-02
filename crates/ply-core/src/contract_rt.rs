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
/// alone -- only their *scalar* leaves get cast to i128.
pub(crate) fn widen(expr: &Expr) -> proc_macro2::TokenStream {
    match expr {
        Expr::Binary(bin) => {
            let op = bin.op;
            match op {
                BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_) | BinOp::Rem(_) => {
                    let l = widen_leaf(&bin.left);
                    let r = widen_leaf(&bin.right);
                    quote::quote!((#l #op #r))
                }
                BinOp::Eq(_)
                | BinOp::Ne(_)
                | BinOp::Lt(_)
                | BinOp::Le(_)
                | BinOp::Gt(_)
                | BinOp::Ge(_) => {
                    let l = widen_leaf(&bin.left);
                    let r = widen_leaf(&bin.right);
                    quote::quote!((#l) #op (#r))
                }
                BinOp::And(_) | BinOp::Or(_) => {
                    let l = widen(&bin.left);
                    let r = widen(&bin.right);
                    quote::quote!((#l) #op (#r))
                }
                _ => expr.to_token_stream(),
            }
        }
        Expr::Paren(p) => widen(&p.expr),
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
/// for arithmetic.
fn widen_leaf(expr: &Expr) -> proc_macro2::TokenStream {
    match expr {
        Expr::Binary(bin)
            if matches!(
                bin.op,
                BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_) | BinOp::Rem(_)
            ) =>
        {
            widen(expr)
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
            let inner = widen(expr);
            quote::quote!(((#inner) as i128))
        }
        Expr::Paren(p) => widen_leaf(&p.expr),
        other => quote::quote!((#other as i128)),
    }
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

    let widened = widen(&checked_body);
    let widened_str = widened.to_string();

    let test_name = format!("ply_cex_{}_{:02}", cf.ident(), index);

    let message = render_message(cf, &checked_body, contract_text, diagnostic_code)?;

    let source = format!(
        "// Reproduces the counterexample for `{fname}` found by check\n\
         // `{check_label}` (diagnostic {code}). This test fails until the\n\
         // function body or its contract change, and passes once they agree.\n\
         #[cfg(test)]\n\
         #[test]\n\
         fn {test_name}() {{\n\
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
/// D7 plan's own fallback clause -- never a bare, uninterpreted panic.
fn render_message(cf: &ContractFn, body: &Expr, contract_text: &str, code: &str) -> Result<String> {
    let fname = &cf.path;
    if let Expr::Binary(bin) = body
        && matches!(
            bin.op,
            BinOp::Eq(_) | BinOp::Ne(_) | BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_)
        )
    {
        let l = widen_leaf(&bin.left).to_string();
        let r = widen_leaf(&bin.right).to_string();
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
