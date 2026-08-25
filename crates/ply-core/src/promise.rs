//! Does a declared promise say anything at all? (The-Ply-Spec.md §5.5.)
//!
//! §5.5's second branch lets a `ply.yaml` entry declare a contract for a
//! callee nothing has verified, and proves the caller against that promise
//! instead of the callee's body. The promise is therefore load-bearing: it
//! is the *only* thing standing between the caller's verdict and code
//! nobody has looked at. Under the design Ply is built for, a language
//! model writes one of these for each piece of old code a new feature
//! calls — so a promise that constrains nothing is not a hypothetical, it
//! is the realistic failure.
//!
//! Two shapes, and they fail differently:
//!
//! - **Unsatisfiable** — no value can satisfy it. Ply hands the clause to
//!   the engine as an assumption, so the caller's proof holds *vacuously*
//!   and anything at all is provable under it. Measured 2026-08-25 on
//!   `tests/fixtures/emptypromise`: a caller whose postcondition is plainly
//!   false came back `bounded(2)`, exit 0.
//! - **Trivially true** — true of every value the type can hold. It
//!   constrains nothing, so the callee is in effect replaced by an
//!   arbitrary value. The caller's verdict is real (an unconstrained value
//!   is the *weaker* assumption, not the stronger one), but the report
//!   called it an assumption owed evidence, which sends a reader off to
//!   discharge a debt that does not exist.
//!
//! Both are decided by asking the engine already in use two questions about
//! the clause alone, with no function body anywhere in the harness:
//!
//! | harness | asks | verified means |
//! |---|---|---|
//! | `ply_promise_sat_*` | `assert(!(c1 && .. && cn))` | nothing satisfies the promise → **unsatisfiable** |
//! | `ply_promise_taut_*` | `assert(ci)` | nothing violates the clause → **trivially true** |
//!
//! These are exhaustive over the value space, not sampled: CBMC solves them
//! symbolically. Measured 2026-08-25: six such harnesses in one `cargo kani`
//! invocation, 3.9s total; 0.43s each once the crate is compiled.
//!
//! What this module cannot decide is stated where a user meets it, not only
//! here: a clause over parameter types Ply's codegen cannot build a
//! `kani::any()` for is reported as unchecked (`W0514`), never as sound.

use quote::ToTokens;
use syn::{Expr, ExprClosure};

use crate::harness::StubSpec;

/// Which half of a declared contract a clause came from. The two need
/// different words: an `ensures` is what the callee promises, a `requires`
/// is what the caller is asked to establish before calling it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseKind {
    Requires,
    Ensures,
}

impl ClauseKind {
    pub fn key(&self) -> &'static str {
        match self {
            ClauseKind::Requires => "requires",
            ClauseKind::Ensures => "ensures",
        }
    }
}

/// The question one generated harness answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    /// `assert(!(the whole promise))`. Verified ⇒ nothing satisfies it.
    Satisfiable,
    /// `assert(one clause)`. Verified ⇒ nothing violates it.
    Violable,
}

/// One generated `#[kani::proof]` that asks one question about one promise.
#[derive(Debug, Clone)]
pub struct PromiseHarness {
    /// The generated fn's name, and therefore the `--harness` path's tail.
    pub fn_name: String,
    pub callee: String,
    pub kind: ClauseKind,
    pub question: Question,
    /// The clause text a user has to read and fix. For `Satisfiable` over
    /// several clauses this is all of them, joined with ` && `.
    pub clause: String,
    /// The type the question ranges over, named the way a user would name
    /// it (`u32`, or `tier: u8, floor: u32`). What makes `>= 0` obviously
    /// empty rather than merely suspicious.
    pub domain: String,
    pub source: String,
}

/// A promise Ply could not ask about, and the plain reason why.
#[derive(Debug, Clone)]
pub struct NotChecked {
    pub callee: String,
    pub kind: ClauseKind,
    pub reason: String,
}

/// Everything the promise gate will run for one caller's stubs.
#[derive(Debug, Clone, Default)]
pub struct PromisePlan {
    pub harnesses: Vec<PromiseHarness>,
    pub not_checked: Vec<NotChecked>,
}

impl PromisePlan {
    pub fn is_empty(&self) -> bool {
        self.harnesses.is_empty() && self.not_checked.is_empty()
    }

    /// The generated source for every harness, ready to append to the
    /// proof module.
    pub fn source(&self) -> String {
        let mut out = String::new();
        for h in &self.harnesses {
            out.push_str(&h.source);
            out.push('\n');
        }
        out
    }
}

/// Builds the promise-content harnesses for every callee `stubs` stands in
/// for. A callee contributes one satisfiability harness per contract half
/// it declares, plus one triviality harness per clause.
pub fn plan(stubs: &[StubSpec]) -> PromisePlan {
    let mut plan = PromisePlan::default();
    for s in stubs {
        plan_ensures(s, &mut plan);
        plan_requires(s, &mut plan);
    }
    plan
}

fn ident_of(callee: &str) -> String {
    callee.replace("::", "_")
}

fn plan_ensures(s: &StubSpec, plan: &mut PromisePlan) {
    if s.ensures.is_empty() {
        return;
    }
    let ret = &s.return_type;
    let mut applied = Vec::new();
    for e in &s.ensures {
        match apply_ensures(e, ret) {
            Ok(expr) => applied.push((e.clone(), expr)),
            Err(reason) => {
                plan.not_checked.push(NotChecked {
                    callee: s.callee_path.clone(),
                    kind: ClauseKind::Ensures,
                    reason,
                });
                return;
            }
        }
    }
    let ident = ident_of(&s.callee_path);
    let binding = format!("    let __ply_result: {ret} = kani::any();\n");

    let conjunction: Vec<String> = applied.iter().map(|(_, a)| a.clone()).collect();
    let clause_text: Vec<String> = applied.iter().map(|(t, _)| t.clone()).collect();
    plan.harnesses.push(PromiseHarness {
        fn_name: format!("ply_promise_sat_{ident}_ensures"),
        callee: s.callee_path.clone(),
        kind: ClauseKind::Ensures,
        question: Question::Satisfiable,
        clause: clause_text.join(" && "),
        domain: ret.clone(),
        source: render(
            &format!("ply_promise_sat_{ident}_ensures"),
            &binding,
            &format!("!({})", conjunction.join(" && ")),
        ),
    });
    for (i, (text, applied_expr)) in applied.iter().enumerate() {
        plan.harnesses.push(PromiseHarness {
            fn_name: format!("ply_promise_taut_{ident}_ensures_{i:02}"),
            callee: s.callee_path.clone(),
            kind: ClauseKind::Ensures,
            question: Question::Violable,
            clause: text.clone(),
            domain: ret.clone(),
            source: render(
                &format!("ply_promise_taut_{ident}_ensures_{i:02}"),
                &binding,
                applied_expr,
            ),
        });
    }
}

fn plan_requires(s: &StubSpec, plan: &mut PromisePlan) {
    if s.requires.is_empty() {
        return;
    }
    // A `requires` ranges over the callee's parameters, so the harness has
    // to build one of each. Unlike the `ensures` case there is no existing
    // guarantee that it can: the stub receives its parameters, it does not
    // invent them. A parameter type Ply's codegen has no `kani::any()` for
    // is reported as unchecked rather than guessed at.
    let mut binding = String::new();
    let mut domain = Vec::new();
    for (name, ty_src) in &s.params {
        match crate::harness::rust_type_from_source(ty_src).and_then(|t| {
            t.is_bounded_supported()
                .then(|| t.rust_name())
                .flatten()
                .map(|n| (t, n))
        }) {
            Some((_, rust_name)) => {
                binding.push_str(&format!("    let {name}: {rust_name} = kani::any();\n"));
                domain.push(format!("{name}: {rust_name}"));
            }
            None => {
                plan.not_checked.push(NotChecked {
                    callee: s.callee_path.clone(),
                    kind: ClauseKind::Requires,
                    reason: format!(
                        "its parameter `{name}` has type `{ty_src}`, and Ply's bounded codegen \
                         cannot build an arbitrary value of that type, so there is nothing to \
                         range the question over"
                    ),
                });
                return;
            }
        }
    }
    let mut exprs = Vec::new();
    for r in &s.requires {
        match syn::parse_str::<Expr>(r) {
            Ok(expr) => exprs.push((r.clone(), format!("({})", expr.to_token_stream()))),
            Err(e) => {
                plan.not_checked.push(NotChecked {
                    callee: s.callee_path.clone(),
                    kind: ClauseKind::Requires,
                    reason: format!("Ply could not read `{r}` as a Rust expression ({e})"),
                });
                return;
            }
        }
    }
    let ident = ident_of(&s.callee_path);
    let domain = domain.join(", ");
    let conjunction: Vec<String> = exprs.iter().map(|(_, a)| a.clone()).collect();
    plan.harnesses.push(PromiseHarness {
        fn_name: format!("ply_promise_sat_{ident}_requires"),
        callee: s.callee_path.clone(),
        kind: ClauseKind::Requires,
        question: Question::Satisfiable,
        clause: exprs
            .iter()
            .map(|(t, _)| t.clone())
            .collect::<Vec<_>>()
            .join(" && "),
        domain: domain.clone(),
        source: render(
            &format!("ply_promise_sat_{ident}_requires"),
            &binding,
            &format!("!({})", conjunction.join(" && ")),
        ),
    });
    for (i, (text, applied)) in exprs.iter().enumerate() {
        plan.harnesses.push(PromiseHarness {
            fn_name: format!("ply_promise_taut_{ident}_requires_{i:02}"),
            callee: s.callee_path.clone(),
            kind: ClauseKind::Requires,
            question: Question::Violable,
            clause: text.clone(),
            domain: domain.clone(),
            source: render(
                &format!("ply_promise_taut_{ident}_requires_{i:02}"),
                &binding,
                applied,
            ),
        });
    }
}

/// `|result| expr` applied to an arbitrary value of the return type. The
/// closure parameter needs its type written out for the same reason the
/// stub's own `kani::assume` does: applied to a reference with nothing else
/// to infer from, rustc reports "type annotations needed".
fn apply_ensures(clause: &str, ret: &str) -> Result<String, String> {
    let closure: ExprClosure = syn::parse_str(clause)
        .map_err(|e| format!("Ply could not read `{clause}` as a `|result| expr` closure ({e})"))?;
    let Some(pat) = closure.inputs.first() else {
        return Err(format!(
            "`{clause}` takes no parameter -- an `ensures` must be a `|result| expr` closure"
        ));
    };
    Ok(format!(
        "((|{pat}: &{ret}| {body})(&__ply_result))",
        pat = pat.to_token_stream(),
        body = closure.body.to_token_stream()
    ))
}

fn render(fn_name: &str, binding: &str, condition: &str) -> String {
    format!(
        "#[cfg(kani)]\n\
         #[allow(dead_code, unused_variables, unused_comparisons)]\n\
         #[kani::proof]\n\
         fn {fn_name}() {{\n\
         {binding}\
         \x20\x20\x20\x20kani::assert({condition}, \"ply promise-content probe\");\n\
         }}\n"
    )
}

/// What running one promise harness said. `Holds` means the engine proved
/// the assertion for every value; `Refuted` means it produced a value that
/// breaks it. `Undecided` is neither, and never silently becomes either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessAnswer {
    Holds,
    Refuted,
    Undecided(String),
}

/// What Ply now knows about one declared clause (or, for `Unsatisfiable`,
/// about one whole half of one callee's contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClauseVerdict {
    /// No value satisfies it. Assuming it makes the caller's proof hold
    /// vacuously, so the proof must not be run.
    Unsatisfiable,
    /// Every value satisfies it. It constrains nothing.
    TriviallyTrue,
    /// Some values satisfy it and some do not — it says something.
    Meaningful,
    /// Ply could not tell, and says so rather than assuming either.
    Undecided(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseFinding {
    pub callee: String,
    pub kind: ClauseKind,
    pub clause: String,
    pub domain: String,
    pub verdict: ClauseVerdict,
}

/// Turns harness answers into findings. Pure, so the rule that decides what
/// a green harness *means* is testable without a subprocess anywhere near
/// it — the place a vacuity check could most easily be wrong in the
/// reassuring direction.
pub fn findings(
    plan: &PromisePlan,
    mut ask: impl FnMut(&PromiseHarness) -> HarnessAnswer,
) -> Vec<PromiseFinding> {
    let mut out: Vec<PromiseFinding> = Vec::new();
    let mut dead: Vec<(String, ClauseKind)> = Vec::new();

    for h in plan
        .harnesses
        .iter()
        .filter(|h| h.question == Question::Satisfiable)
    {
        let verdict = match ask(h) {
            // The assertion `!(promise)` held for every value, so no value
            // satisfies the promise.
            HarnessAnswer::Holds => {
                dead.push((h.callee.clone(), h.kind));
                ClauseVerdict::Unsatisfiable
            }
            // A value satisfying the promise exists. That is the ordinary
            // case and earns no finding of its own.
            HarnessAnswer::Refuted => continue,
            HarnessAnswer::Undecided(why) => ClauseVerdict::Undecided(why),
        };
        out.push(PromiseFinding {
            callee: h.callee.clone(),
            kind: h.kind,
            clause: h.clause.clone(),
            domain: h.domain.clone(),
            verdict,
        });
    }

    for h in plan
        .harnesses
        .iter()
        .filter(|h| h.question == Question::Violable)
    {
        // A clause inside a promise nothing can satisfy is already reported
        // as part of that promise; saying it is also trivially true (which
        // it may well be, over a space with no values in it) would be two
        // sentences for one defect.
        if dead.contains(&(h.callee.clone(), h.kind)) {
            continue;
        }
        let verdict = match ask(h) {
            // The clause held for every value: nothing violates it.
            HarnessAnswer::Holds => ClauseVerdict::TriviallyTrue,
            HarnessAnswer::Refuted => ClauseVerdict::Meaningful,
            HarnessAnswer::Undecided(why) => ClauseVerdict::Undecided(why),
        };
        if verdict == ClauseVerdict::Meaningful {
            continue;
        }
        out.push(PromiseFinding {
            callee: h.callee.clone(),
            kind: h.kind,
            clause: h.clause.clone(),
            domain: h.domain.clone(),
            verdict,
        });
    }

    for n in &plan.not_checked {
        out.push(PromiseFinding {
            callee: n.callee.clone(),
            kind: n.kind,
            clause: String::new(),
            domain: String::new(),
            verdict: ClauseVerdict::Undecided(n.reason.clone()),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub(requires: &[&str], ensures: &[&str]) -> StubSpec {
        StubSpec {
            callee_path: "rates::legacy_rate".into(),
            params: vec![("tier".into(), "u8".into())],
            return_type: "u32".into(),
            requires: requires.iter().map(|s| s.to_string()).collect(),
            ensures: ensures.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn one_ensures_clause_asks_both_questions_over_the_return_type() {
        let plan = plan(&[stub(&[], &["|result| *result <= 10_000"])]);
        let names: Vec<&str> = plan.harnesses.iter().map(|h| h.fn_name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "ply_promise_sat_rates_legacy_rate_ensures",
                "ply_promise_taut_rates_legacy_rate_ensures_00"
            ]
        );
        assert!(plan.not_checked.is_empty());
        let sat = &plan.harnesses[0];
        assert_eq!(sat.domain, "u32");
        assert!(
            sat.source.contains("let __ply_result : u32 = kani::any();")
                || sat.source.contains("let __ply_result: u32 = kani::any();"),
            "{}",
            sat.source
        );
        assert!(
            sat.source.contains("kani::assert(!("),
            "the satisfiability question is the negation of the whole promise: {}",
            sat.source
        );
    }

    /// The question that catches vacuity has to be about the promise as a
    /// whole: two clauses can each be satisfiable and still contradict each
    /// other, and it is their conjunction the stub assumes.
    #[test]
    fn several_ensures_clauses_are_asked_about_as_one_conjunction() {
        let plan = plan(&[stub(
            &[],
            &["|result| *result > 10_000", "|result| *result < 5"],
        )]);
        let sat = plan
            .harnesses
            .iter()
            .find(|h| h.question == Question::Satisfiable)
            .unwrap();
        assert_eq!(
            sat.clause,
            "|result| *result > 10_000 && |result| *result < 5"
        );
        assert_eq!(
            sat.source.matches("__ply_result").count(),
            3,
            "one binding plus one application per clause: {}",
            sat.source
        );
        assert_eq!(
            plan.harnesses
                .iter()
                .filter(|h| h.question == Question::Violable)
                .count(),
            2,
            "triviality is per clause -- one empty clause beside a real one is still an empty \
             clause a user wrote and should be told about"
        );
    }

    #[test]
    fn a_requires_clause_ranges_over_the_callees_parameters() {
        let plan = plan(&[stub(&["tier < 4"], &[])]);
        let sat = plan
            .harnesses
            .iter()
            .find(|h| h.question == Question::Satisfiable)
            .unwrap();
        assert_eq!(sat.kind, ClauseKind::Requires);
        assert_eq!(sat.domain, "tier: u8");
        assert!(
            sat.source.contains("let tier: u8 = kani::any();"),
            "{}",
            sat.source
        );
    }

    /// The honest limit, and it must be visible rather than silently
    /// skipped: a `requires` over a type the bounded codegen has no
    /// arbitrary value for cannot be asked about at all.
    #[test]
    fn a_requires_over_an_unsupported_parameter_type_is_reported_unchecked() {
        let mut s = stub(&["xs.len() > 0"], &[]);
        s.params = vec![("xs".into(), "& Vec < String >".into())];
        let plan = plan(&[s]);
        assert!(plan.harnesses.is_empty());
        assert_eq!(plan.not_checked.len(), 1);
        let n = &plan.not_checked[0];
        assert_eq!(n.kind, ClauseKind::Requires);
        assert!(n.reason.contains("xs"), "{}", n.reason);
        assert!(
            n.reason.contains("Vec"),
            "the type must be named, so a reader knows what to change: {}",
            n.reason
        );
    }

    #[test]
    fn a_callee_with_no_declared_clauses_generates_nothing() {
        assert!(plan(&[stub(&[], &[])]).is_empty());
    }

    // -- what a green harness means. The one place this check could be
    // wrong in the reassuring direction, so it is decided by a pure
    // function with no subprocess anywhere near it.

    fn answer_all(plan: &PromisePlan, a: HarnessAnswer) -> Vec<PromiseFinding> {
        findings(plan, |_| a.clone())
    }

    #[test]
    fn a_promise_nothing_satisfies_is_unsatisfiable() {
        // `assert(!(promise))` holding for every value is exactly "no value
        // satisfies the promise".
        let plan = plan(&[stub(&[], &["|result| *result > 10_000"])]);
        let found = answer_all(&plan, HarnessAnswer::Holds);
        assert_eq!(
            found
                .iter()
                .filter(|f| f.verdict == ClauseVerdict::Unsatisfiable)
                .count(),
            1
        );
        assert!(
            !found
                .iter()
                .any(|f| f.verdict == ClauseVerdict::TriviallyTrue),
            "one defect, one sentence: a clause inside an impossible promise is not separately              reported as trivially true"
        );
    }

    #[test]
    fn a_promise_nothing_violates_is_trivially_true() {
        let plan = plan(&[stub(&[], &["|result| *result >= 0"])]);
        // Satisfiable (its negation is refuted), and nothing violates it.
        let found = findings(&plan, |h| match h.question {
            Question::Satisfiable => HarnessAnswer::Refuted,
            Question::Violable => HarnessAnswer::Holds,
        });
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].verdict, ClauseVerdict::TriviallyTrue);
        assert_eq!(found[0].domain, "u32");
    }

    #[test]
    fn a_promise_that_says_something_produces_no_finding_at_all() {
        let plan = plan(&[stub(&["tier < 4"], &["|result| *result <= 10_000"])]);
        assert!(answer_all(&plan, HarnessAnswer::Refuted).is_empty());
    }

    /// An engine that could not answer must never round to "fine". This is
    /// the same discipline §5.4c holds the Kani adapter to: a timeout is a
    /// different kind of fact from a pass, not a weaker one.
    #[test]
    fn an_engine_that_could_not_answer_is_undecided_not_meaningful() {
        let plan = plan(&[stub(&[], &["|result| *result <= 10_000"])]);
        let found = answer_all(&plan, HarnessAnswer::Undecided("CBMC timed out".into()));
        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .all(|f| matches!(f.verdict, ClauseVerdict::Undecided(_))),
            "{found:?}"
        );
    }
}
