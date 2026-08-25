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
}
