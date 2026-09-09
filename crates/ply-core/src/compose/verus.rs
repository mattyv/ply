//! The Verus adapter: turning a plan into obligations a solver can answer.
//!
//! Backend-specific code lives here and nowhere else. The planner
//! (`super`) knows nothing about Verus, and this module makes no decisions
//! about what must be proved -- it only encodes what it is handed.
//!
//! **The arithmetic rule this module exists to enforce.** A Ply contract is
//! a Rust expression, so `old(available) - tokens` is *machine*
//! subtraction. Transcribing it into the prover's unbounded `int` changes
//! its meaning, and the first version of this work did exactly that: an
//! implementation using `wrapping_sub` satisfies the promise while breaking
//! the invariant, and the model verified anyway
//! (`tests/spike/verus-component/compose/wrap_counterexample.rs`).
//!
//! So every arithmetic operation in a translated clause carries a range
//! obligation. A clause that cannot discharge it is named and refused,
//! never quietly reinterpreted under wider arithmetic.
//!
//! **Two things in here read backwards, and both are deliberate.**
//!
//! The range obligation must not assume the state after the operation is
//! in range. That looks like a free premise and is in fact the whole hole:
//! wrapping arithmetic produces a perfectly in-range value, so assuming it
//! makes the premise set contradictory and every question about that
//! operation answers yes. Measured: with that premise present the
//! unguarded contract passes; without it, it fails as it should.
//!
//! And the satisfiability probe asks the solver to derive a falsehood from
//! the contract alone. It *succeeding* is the bad news -- it means the
//! contract contradicts itself, and every other result about that
//! operation is worthless.

use std::collections::BTreeMap;

use super::{ObligationKind, Parameter, Plan, Premise, PremiseRole, Property, RangeFacts};

/// A clause this adapter will not translate, and why. Refusing by name is
/// the point: silently dropping a clause would make the remaining proof
/// look complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Untranslatable {
    pub item: String,
    pub clause: String,
    pub reason: String,
}

/// One arithmetic operation that must be shown in range, in the terms a
/// reader of the report would need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeCheck {
    pub clause: String,
    /// The Rust-level operation, e.g. `pre.available - tokens`.
    pub operation: String,
    /// The condition that keeps it inside the declared type.
    pub condition: String,
    /// What has to hold for the operation to happen at all. `a && b` and
    /// `a || b` only evaluate their right side under a condition, and an
    /// implication only asserts its consequent under its antecedent -- so
    /// an operation on the guarded side owes its range only there. An
    /// unguarded obligation would be unprovable for a contract that is
    /// perfectly safe.
    pub guard: Option<String>,
}

/// What the adapter produced, and everything it could not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Encoded {
    /// Every obligation whose success is success. A run of this must come
    /// back with no errors.
    pub source: String,
    /// The satisfiability probes, kept in a file of their own because
    /// their success is the failure. A run of this must come back with an
    /// error for *every* obligation in it; a single one verifying means
    /// that operation's contract contradicts itself and every other answer
    /// about it is worthless. Mixed into one file, a reader counting
    /// errors would have to know which lines invert -- so they do not
    /// share a file.
    pub probe: String,
    /// Clauses the adapter would not translate. Non-empty means the
    /// encoding is not the whole contract.
    pub refused: Vec<Untranslatable>,
    /// Obligations written into `source`, by the name they carry there.
    pub emitted: Vec<String>,
    /// Obligations that could not be written, and why. A solver success
    /// says nothing about these.
    pub omitted: Vec<(String, String)>,
    /// Obligations with nothing to check, and why. Not a failure and not a
    /// result -- kept separate from `omitted` so the two are never read as
    /// the same thing.
    pub trivial: Vec<(String, String)>,
    /// Obligations whose success is the failure. See the module note.
    pub inverted: Vec<String>,
}

/// The name an obligation carries in the emitted source, so a solver
/// result can be matched back to what was asked.
pub fn obligation_name(kind: &ObligationKind) -> String {
    match kind {
        ObligationKind::Initialization { constructor } => format!("ob_init_{}", sym(constructor)),
        ObligationKind::Preservation { operation } => format!("ob_preserve_{}", sym(operation)),
        ObligationKind::Reachable { operation } => format!("ob_reach_{}", sym(operation)),
        ObligationKind::ArithmeticSafety { item } => format!("ob_arith_{}", sym(item)),
    }
}

/// An item path as an identifier the backend will accept.
///
/// Not injective -- `A::b` and `A_b` both land on `A_b` -- so `encode`
/// checks for a collision and refuses both rather than letting one item's
/// obligations answer for another's.
fn sym(item: &str) -> String {
    item.replace("::", "_")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Which state a bare reading refers to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bare {
    /// In a postcondition, a bare reading is the state *after*. `old(..)`
    /// is the only way to reach the state before, so a frame fact cannot
    /// hold by accident.
    Post,
    /// In a precondition there is no state after, so a bare reading is the
    /// state before and `old(..)` is a mistake.
    Pre,
}

/// Translate one postcondition clause into the adapter's spec language, or
/// refuse it by name.
///
/// Supported, deliberately narrowly: readings and parameter names, integer
/// literals, `old(..)`, `min(a, b)`, `max(a, b)`, `!`, the comparison and
/// equality operators, `&&`, `||`, `==>`, `+ - *`, and parentheses.
/// Everything else is refused -- including other calls, field access,
/// casts and indexing -- because a clause this adapter half-understands is
/// worse than one it declines.
pub fn translate_clause(
    clause: &str,
    property: &Property,
    parameters: &[Parameter],
) -> Result<String, Untranslatable> {
    translate(
        clause,
        property,
        &environment(property, parameters),
        Bare::Post,
    )
}

fn translate(
    clause: &str,
    property: &Property,
    env: &Env,
    bare: Bare,
) -> Result<String, Untranslatable> {
    let expr = parse(clause)?;
    let mut out = String::new();
    render(&expr, property, env, clause, bare, &mut out)?;
    Ok(out)
}

type Env = BTreeMap<String, RangeFacts>;

/// Parse a contract clause.
///
/// `==>` is not Rust, so it is read as `=`: assignment is the one Rust
/// operator with implication's precedence (lower than everything) and its
/// associativity (right). The first attempt swapped in `|` instead, which
/// binds *tighter* than `==` and left-associates, so
/// `ok ==> available == old(available) - tokens` came apart as
/// `((ok | IMPLIES) | available) == ...` -- a stray name, the wrong
/// grouping, and no test noticed because they all checked only that
/// translation succeeded (2026-09-09).
fn parse(clause: &str) -> Result<syn::Expr, Untranslatable> {
    let refuse = |reason: String| Untranslatable {
        item: String::new(),
        clause: clause.to_string(),
        reason,
    };
    if has_bare_assignment(clause) {
        return Err(refuse(
            "it contains `=`, which a contract cannot mean -- a clause states what is true,              it does not assign"
                .into(),
        ));
    }
    syn::parse_str(&clause.replace("==>", "="))
        .map_err(|e| refuse(format!("it does not parse as a Rust expression ({e})")))
}

/// An `=` that is not part of `==`, `!=`, `<=`, `>=` or `==>`.
fn has_bare_assignment(clause: &str) -> bool {
    let b: Vec<char> = clause.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '=' {
            let prev = i.checked_sub(1).map(|j| b[j]);
            let next = b.get(i + 1).copied();
            let paired =
                matches!(prev, Some('=') | Some('!') | Some('<') | Some('>')) || next == Some('=');
            if !paired {
                return true;
            }
            // Step past a two-character operator so `a == b` is not read as
            // `a =` followed by a bare `= b`.
            if next == Some('=') {
                i += 1;
            }
        }
        i += 1;
    }
    false
}

fn render(
    e: &syn::Expr,
    property: &Property,
    env: &Env,
    clause: &str,
    bare: Bare,
    out: &mut String,
) -> Result<(), Untranslatable> {
    let refuse = |what: &str| Untranslatable {
        item: String::new(),
        clause: clause.to_string(),
        reason: format!("it contains {what}, which this adapter does not translate"),
    };
    match e {
        syn::Expr::Path(p) => {
            let name = last_segment(p);
            if property.observers.iter().any(|o| o.name == name) {
                let state = match bare {
                    Bare::Post => "post",
                    Bare::Pre => "pre",
                };
                out.push_str(&format!("{state}.{name}"));
            } else {
                out.push_str(&name);
            }
            Ok(())
        }
        syn::Expr::Lit(l) => {
            out.push_str(&quote_lit(l));
            Ok(())
        }
        syn::Expr::Paren(p) => {
            out.push('(');
            render(&p.expr, property, env, clause, bare, out)?;
            out.push(')');
            Ok(())
        }
        syn::Expr::Unary(u) => match u.op {
            syn::UnOp::Not(_) => {
                out.push('!');
                render(&u.expr, property, env, clause, bare, out)
            }
            _ => Err(refuse("a unary operator other than `!`")),
        },
        syn::Expr::Binary(b) => {
            let Some(op) = binop(&b.op) else {
                return Err(refuse("an operator this adapter does not translate"));
            };
            render(&b.left, property, env, clause, bare, out)?;
            out.push_str(&format!(" {op} "));
            render(&b.right, property, env, clause, bare, out)
        }
        syn::Expr::Call(c) => {
            let syn::Expr::Path(p) = &*c.func else {
                return Err(refuse("a call through an expression"));
            };
            let callee = last_segment(p);
            match (callee.as_str(), c.args.len()) {
                // The pre-state reading.
                ("old", 1) => {
                    if bare == Bare::Pre {
                        return Err(Untranslatable {
                            item: String::new(),
                            clause: clause.to_string(),
                            reason: "it reads `old(..)` where there is no earlier state to read \
                                     -- a precondition and a constructor both describe one state \
                                     only"
                                .into(),
                        });
                    }
                    let syn::Expr::Path(inner) = &c.args[0] else {
                        return Err(refuse("`old(..)` of something other than a plain reading"));
                    };
                    out.push_str(&format!("pre.{}", last_segment(inner)));
                    Ok(())
                }
                // A fixed two-entry table, not the start of a general
                // translator: both have an exact meaning over integers,
                // and both appear in contracts people actually write.
                // Written out rather than called, so this does not depend
                // on what the backend's library happens to name them.
                ("min", 2) | ("max", 2) => {
                    let cmp = if callee == "min" { "<=" } else { ">=" };
                    let (mut a, mut b) = (String::new(), String::new());
                    render(&c.args[0], property, env, clause, bare, &mut a)?;
                    render(&c.args[1], property, env, clause, bare, &mut b)?;
                    out.push_str(&format!("(if {a} {cmp} {b} {{ {a} }} else {{ {b} }})"));
                    Ok(())
                }
                // Total where `+` is partial, which is why the real
                // `refill` is written with it -- and why it owes no range
                // obligation: there is no input on which it misbehaves.
                // Its ceiling comes from the operands' declared type, so
                // it is only translatable where that type is known.
                ("saturating_add", 2) => {
                    let Some((_, _, max)) = width_of(e, env, clause)? else {
                        return Err(refuse(
                            "a saturating addition of values with no declared width, so there \
                             is no ceiling to saturate at",
                        ));
                    };
                    let (mut a, mut b) = (String::new(), String::new());
                    render(&c.args[0], property, env, clause, bare, &mut a)?;
                    render(&c.args[1], property, env, clause, bare, &mut b)?;
                    out.push_str(&format!(
                        "(if {a} + {b} <= {max} {{ {a} + {b} }} else {{ {max} }})"
                    ));
                    Ok(())
                }
                _ => Err(refuse(&format!("a call to `{callee}`"))),
            }
        }
        // `==>`, read back from the assignment it was parsed as.
        syn::Expr::Assign(a) => {
            out.push('(');
            render(&a.left, property, env, clause, bare, out)?;
            out.push_str(") ==> (");
            render(&a.right, property, env, clause, bare, out)?;
            out.push(')');
            Ok(())
        }
        syn::Expr::MethodCall(_) => Err(refuse("a method call")),
        syn::Expr::Field(_) => Err(refuse("field access")),
        syn::Expr::Cast(_) => Err(refuse("a cast")),
        syn::Expr::Index(_) => Err(refuse("indexing")),
        syn::Expr::If(_) => Err(refuse("an `if` expression")),
        _ => Err(refuse("an expression shape it was not built for")),
    }
}

fn binop(op: &syn::BinOp) -> Option<&'static str> {
    Some(match op {
        syn::BinOp::Add(_) => "+",
        syn::BinOp::Sub(_) => "-",
        syn::BinOp::Mul(_) => "*",
        syn::BinOp::Eq(_) => "==",
        syn::BinOp::Ne(_) => "!=",
        syn::BinOp::Lt(_) => "<",
        syn::BinOp::Le(_) => "<=",
        syn::BinOp::Gt(_) => ">",
        syn::BinOp::Ge(_) => ">=",
        syn::BinOp::And(_) => "&&",
        syn::BinOp::Or(_) => "||",
        _ => return None,
    })
}

fn last_segment(p: &syn::ExprPath) -> String {
    p.path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

fn quote_lit(l: &syn::ExprLit) -> String {
    match &l.lit {
        syn::Lit::Int(i) => i.base10_digits().to_string(),
        syn::Lit::Bool(b) => b.value.to_string(),
        other => format!("{other:?}"),
    }
}

/// Every arithmetic operation in a clause, with the condition that keeps it
/// inside its declared type and the guard it happens under.
///
/// This is the whole reason the adapter exists in this shape. Without it,
/// `old(available) - tokens` is unbounded subtraction in the model and
/// wrapping subtraction in the program, and the two disagree exactly where
/// the invariant breaks.
pub fn range_checks(
    clause: &str,
    property: &Property,
    parameters: &[Parameter],
) -> Result<Vec<RangeCheck>, Untranslatable> {
    let expr = parse(clause)?;
    let env = environment(property, parameters);
    let mut out = Vec::new();
    walk_arith(&expr, property, &env, clause, &mut Vec::new(), &mut out)?;
    Ok(out)
}

/// The one translation helper the arithmetic walk needs, already holding a
/// type environment.
fn render_text(text: &str, property: &Property, env: &Env) -> Result<String, Untranslatable> {
    translate(text, property, env, Bare::Post)
}

/// The declared width of every name a clause may mention.
fn environment(property: &Property, parameters: &[Parameter]) -> BTreeMap<String, RangeFacts> {
    let mut env = BTreeMap::new();
    for o in &property.observers {
        env.insert(o.name.clone(), o.facts.clone());
    }
    for p in parameters {
        env.insert(p.name.clone(), p.facts.clone());
    }
    env
}

/// The declared integer type of an expression, or `None` where it has none
/// (a bare literal). `Err` where two widths meet, which means the clause is
/// not a well-typed Rust expression and picking one would silently widen
/// or narrow the program.
fn width_of(
    e: &syn::Expr,
    env: &BTreeMap<String, RangeFacts>,
    clause: &str,
) -> Result<Option<(String, i128, i128)>, Untranslatable> {
    let unify = |a: Option<(String, i128, i128)>,
                 b: Option<(String, i128, i128)>|
     -> Result<Option<(String, i128, i128)>, Untranslatable> {
        match (a, b) {
            (Some(x), Some(y)) if x.0 != y.0 => Err(Untranslatable {
                item: String::new(),
                clause: clause.to_string(),
                reason: format!(
                    "one operation mixes two declared widths, `{}` and `{}`, so there is no \
                     single width to bound it by",
                    x.0, y.0
                ),
            }),
            (Some(x), _) => Ok(Some(x)),
            (_, b) => Ok(b),
        }
    };
    match e {
        syn::Expr::Path(p) => Ok(match env.get(&last_segment(p)) {
            Some(RangeFacts::Integer {
                rust_type,
                min,
                max,
            }) => Some((rust_type.clone(), *min, *max)),
            _ => None,
        }),
        syn::Expr::Paren(p) => width_of(&p.expr, env, clause),
        syn::Expr::Assign(_) => Ok(None),
        syn::Expr::Binary(b) => {
            let l = width_of(&b.left, env, clause)?;
            let r = width_of(&b.right, env, clause)?;
            unify(l, r)
        }
        syn::Expr::Call(c) => {
            let mut acc = None;
            for a in &c.args {
                acc = unify(acc, width_of(a, env, clause)?)?;
            }
            Ok(acc)
        }
        _ => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_arith(
    e: &syn::Expr,
    property: &Property,
    env: &BTreeMap<String, RangeFacts>,
    clause: &str,
    guards: &mut Vec<String>,
    out: &mut Vec<RangeCheck>,
) -> Result<(), Untranslatable> {
    match e {
        syn::Expr::Binary(b) => {
            let arith = matches!(
                b.op,
                syn::BinOp::Add(_) | syn::BinOp::Sub(_) | syn::BinOp::Mul(_)
            );
            if arith {
                let Some((_, min, max)) = width_of(e, env, clause)? else {
                    return Err(Untranslatable {
                        item: String::new(),
                        clause: clause.to_string(),
                        reason: "it performs arithmetic on values with no declared width, so \
                                 there is no range to check it against"
                            .into(),
                    });
                };
                let l = render_text(&expr_text(&b.left), property, env)?;
                let r = render_text(&expr_text(&b.right), property, env)?;
                let sym = match b.op {
                    syn::BinOp::Add(_) => "+",
                    syn::BinOp::Sub(_) => "-",
                    _ => "*",
                };
                let operation = format!("{l} {sym} {r}");
                // For an unsigned type only one end can be reached: a
                // subtraction can go below zero, and a sum or product can
                // pass the top. A signed type can do either, so both ends
                // are stated.
                let condition = if min == 0 {
                    match sym {
                        "-" => format!("{operation} >= 0"),
                        _ => format!("{operation} <= {max}"),
                    }
                } else {
                    format!("{min} <= {operation} <= {max}")
                };
                out.push(RangeCheck {
                    clause: clause.to_string(),
                    operation,
                    condition,
                    guard: (!guards.is_empty()).then(|| guards.join(" && ")),
                });
            }
            // Short-circuiting: the right side of `&&`, `||` and `==>` is
            // only reached under a condition, so anything on it owes its
            // range only there.
            let guard = match b.op {
                syn::BinOp::And(_) => Some(render_text(&expr_text(&b.left), property, env)?),
                syn::BinOp::Or(_) => Some(format!(
                    "!({})",
                    render_text(&expr_text(&b.left), property, env)?
                )),
                _ => None,
            };
            walk_arith(&b.left, property, env, clause, guards, out)?;
            match guard {
                Some(g) => {
                    guards.push(g);
                    let r = walk_arith(&b.right, property, env, clause, guards, out);
                    guards.pop();
                    r
                }
                None => walk_arith(&b.right, property, env, clause, guards, out),
            }
        }
        // An implication asserts its consequent only under its antecedent,
        // so arithmetic on the right owes its range only there.
        syn::Expr::Assign(a) => {
            walk_arith(&a.left, property, env, clause, guards, out)?;
            guards.push(render_text(&expr_text(&a.left), property, env)?);
            let r = walk_arith(&a.right, property, env, clause, guards, out);
            guards.pop();
            r
        }
        syn::Expr::Paren(p) => walk_arith(&p.expr, property, env, clause, guards, out),
        syn::Expr::Unary(u) => walk_arith(&u.expr, property, env, clause, guards, out),
        syn::Expr::Call(c) => {
            for a in &c.args {
                walk_arith(a, property, env, clause, guards, out)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The source text of a sub-expression, good enough to hand back through
/// the translator. Anything this cannot reproduce comes back empty and is
/// refused by the translator rather than guessed at.
fn expr_text(e: &syn::Expr) -> String {
    match e {
        syn::Expr::Path(p) => last_segment(p),
        syn::Expr::Lit(l) => quote_lit(l),
        syn::Expr::Call(c) => {
            let callee = match &*c.func {
                syn::Expr::Path(p) => last_segment(p),
                _ => String::new(),
            };
            let args: Vec<String> = c.args.iter().map(expr_text).collect();
            format!("{callee}({})", args.join(", "))
        }
        syn::Expr::Paren(p) => format!("({})", expr_text(&p.expr)),
        syn::Expr::Unary(u) => match u.op {
            syn::UnOp::Not(_) => format!("!{}", expr_text(&u.expr)),
            _ => String::new(),
        },
        syn::Expr::Binary(b) => match binop(&b.op) {
            Some(op) => format!("({} {op} {})", expr_text(&b.left), expr_text(&b.right)),
            None => String::new(),
        },
        syn::Expr::Assign(a) => {
            format!("({} ==> {})", expr_text(&a.left), expr_text(&a.right))
        }
        _ => String::new(),
    }
}

/// Emit the plan's obligations as a Verus source file.
///
/// Nothing is dropped in silence: every obligation the planner asked for
/// comes back either written into the source, reported as omitted with a
/// reason, or reported as having nothing to check.
pub fn encode(plan: &Plan, premises: &[Premise]) -> Encoded {
    let property = &plan.property;
    let mut e = Encoded::default();
    let by_item: BTreeMap<&str, &Premise> = premises.iter().map(|p| (p.item.as_str(), p)).collect();

    let mut s = String::new();
    s.push_str("// Generated by Ply's composition adapter. Every obligation below is\n");
    s.push_str("// entailment from contracts: no function bodies appear, and none are read.\n");
    s.push_str("// Expected result: no errors.\n");
    // Obligation names carry the Rust item path they are about, which is
    // worth more to a reader than the naming convention.
    s.push_str("#![allow(non_snake_case)]\nuse vstd::prelude::*;\n\nverus! {\n\n");

    s.push_str("/// The readings the property is written in terms of.\n");
    s.push_str("pub struct S {\n");
    for o in &property.observers {
        s.push_str(&format!("    pub {}: int,\n", o.name));
    }
    s.push_str("}\n\n");

    s.push_str("/// The declared Rust widths, carried as premises rather than assumed. A\n");
    s.push_str("/// reading modelled as an *unbounded* integer is a different program.\n");
    s.push_str("pub open spec fn typed(s: S) -> bool {\n    true\n");
    for o in &property.observers {
        if let RangeFacts::Integer { min, max, .. } = &o.facts {
            s.push_str(&format!("    && {min} <= s.{} <= {max}\n", o.name));
        }
    }
    s.push_str("}\n\n");

    match translate_clause(&property.invariant, property, &[]) {
        Ok(inv) => s.push_str(&format!(
            "pub open spec fn inv(s: S) -> bool {{ {} }}\n\n",
            inv.replace("post.", "s.")
        )),
        Err(mut u) => {
            // Without the invariant there is no property, so every
            // obligation goes with it.
            u.item = property.state_type.clone();
            e.refused.push(u);
            for ob in &plan.obligations {
                e.omitted.push((
                    obligation_name(&ob.kind),
                    format!(
                        "the property `{}` could not be translated, so there was nothing to \
                         prove about it",
                        property.invariant
                    ),
                ));
            }
            e.source = s;
            return e;
        }
    }

    // Each item's contract, as one spec function per side.
    let mut usable: BTreeMap<&str, Contract> = BTreeMap::new();
    let mut unusable: BTreeMap<&str, String> = BTreeMap::new();
    let mut items: Vec<&str> = plan
        .obligations
        .iter()
        .map(|o| item_of(&o.kind))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    items.sort();

    // Two items whose paths flatten to the same identifier would share one
    // set of obligations, and the solver's answer about one would be
    // reported for both.
    let mut by_symbol: BTreeMap<String, &str> = BTreeMap::new();
    let mut collided: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for item in &items {
        if let Some(other) = by_symbol.insert(sym(item), item) {
            collided.insert(other);
            collided.insert(item);
        }
    }

    for item in items {
        if collided.contains(item) {
            unusable.insert(
                item,
                format!(
                    "`{item}` and another item flatten to the same name `{}`, so their                      obligations could not be told apart",
                    sym(item)
                ),
            );
            continue;
        }
        let Some(p) = by_item.get(item) else {
            unusable.insert(
                item,
                format!("no premise was supplied for `{item}`, so its contract is not available"),
            );
            continue;
        };
        match contract_of(p, property) {
            Ok(c) => {
                s.push_str(&c.declarations);
                usable.insert(item, c);
            }
            Err(u) => {
                unusable.insert(
                    item,
                    format!(
                        "a clause of `{item}` was not translatable: {} ({})",
                        u.clause, u.reason
                    ),
                );
                e.refused.push(Untranslatable {
                    item: item.to_string(),
                    ..u
                });
            }
        }
    }

    // Everything above is shared by both files.
    let declarations = s.clone();
    let mut probe = declarations.replace(
        "// Expected result: no errors.",
        "// THE SATISFIABILITY PROBES. Each asks the solver to derive a falsehood from one\n         // contract alone. Expected result: an error for EVERY obligation below. One that\n         // verifies is a contract that contradicts itself, and every other answer about\n         // that operation means nothing.",
    );

    for ob in &plan.obligations {
        let name = obligation_name(&ob.kind);
        let item = item_of(&ob.kind);
        let Some(c) = usable.get(item) else {
            let why = unusable
                .get(item)
                .cloned()
                .unwrap_or_else(|| format!("`{item}` has no usable contract"));
            e.omitted.push((name, why));
            continue;
        };
        match &ob.kind {
            ObligationKind::Initialization { .. } => {
                s.push_str(&format!(
                    "/// Every state this construction path can produce satisfies the property.\n\
                     /// The property is deliberately absent from the premises: assuming what is\n\
                     /// to be proved is the shortest route to a solver agreeing with you.\n\
                     proof fn {name}({args})\n    requires\n{req}    ensures inv(post)\n{{ }}\n\n",
                    args = c.signature(false),
                    req = c.requires(Assume::Init),
                ));
                e.emitted.push(name);
            }
            ObligationKind::Preservation { .. } => {
                s.push_str(&format!(
                    "/// The induction step: the property held before, and the contract carries\n\
                     /// it across. `typed(post)` is a premise here and NOT in the range\n\
                     /// obligation below -- there it would be the hole itself.\n\
                     proof fn {name}({args})\n    requires\n{req}    ensures inv(post)\n{{ }}\n\n",
                    args = c.signature(true),
                    req = c.requires(Assume::Preserve),
                ));
                e.emitted.push(name);
            }
            ObligationKind::Reachable { .. } => {
                probe.push_str(&format!(
                    "/// VERIFYING THIS IS THE FAILURE. A contract that contradicts itself\n\
                     /// entails everything, including its own invariant, so a green result\n\
                     /// elsewhere would mean nothing. This asks the solver for a falsehood from\n\
                     /// the contract alone: it must NOT be able to find one. (One-sided -- it\n\
                     /// detects a contradiction, it does not certify there is none.)\n\
                     proof fn {name}({args})\n    requires\n{req}    ensures false\n{{ }}\n\n",
                    args = c.signature(true),
                    req = c.requires(Assume::Reach),
                ));
                e.emitted.push(name.clone());
                e.inverted.push(name);
            }
            ObligationKind::ArithmeticSafety { .. } => {
                if c.range.is_empty() {
                    e.trivial.push((
                        name,
                        format!("the contract of `{item}` performs no arithmetic"),
                    ));
                    continue;
                }
                let ensures: Vec<String> = c
                    .range
                    .iter()
                    .map(|r| match &r.guard {
                        Some(g) => format!("        ({g}) ==> ({})", r.condition),
                        None => format!("        {}", r.condition),
                    })
                    .collect();
                s.push_str(&format!(
                    "/// Rust arithmetic, not the solver's. The contract is a Rust expression,\n\
                     /// so each operation in it has to stay inside the declared type -- an\n\
                     /// implementation that wraps satisfies the promise and breaks the property.\n\
                     /// `typed(post)` is deliberately NOT assumed: a wrapped value is perfectly\n\
                     /// in range, so assuming it makes these premises contradictory and every\n\
                     /// question answers yes.\n\
                     proof fn {name}({args})\n    requires\n{req}    ensures\n{ens}\n{{ }}\n\n",
                    args = c.signature(c.two_state),
                    req = c.requires(Assume::Arith),
                    ens = ensures.join(",\n"),
                ));
                e.emitted.push(name);
            }
        }
    }

    s.push_str("fn main() { }\n}\n");
    probe.push_str("fn main() { }\n}\n");
    e.source = s;
    e.probe = probe;
    e
}

fn item_of(kind: &ObligationKind) -> &str {
    match kind {
        ObligationKind::Initialization { constructor } => constructor,
        ObligationKind::Preservation { operation } => operation,
        ObligationKind::Reachable { operation } => operation,
        ObligationKind::ArithmeticSafety { item } => item,
    }
}

/// What each obligation is entitled to assume.
#[derive(Clone, Copy)]
enum Assume {
    Init,
    Preserve,
    Reach,
    Arith,
}

/// One item's contract, translated once and reused by every obligation
/// about it.
struct Contract {
    declarations: String,
    two_state: bool,
    params: Vec<Parameter>,
    pre_call: Option<String>,
    post_call: String,
    range: Vec<RangeCheck>,
}

impl Contract {
    /// The proof function's parameter list.
    fn signature(&self, two_state: bool) -> String {
        let mut args = Vec::new();
        if two_state {
            args.push("pre: S".to_string());
        }
        args.push("post: S".to_string());
        for p in &self.params {
            args.push(format!("{}: {}", p.name, spec_type(&p.facts)));
        }
        args.join(", ")
    }

    fn requires(&self, assume: Assume) -> String {
        let mut lines = Vec::new();
        match assume {
            Assume::Init => lines.push("typed(post)".into()),
            Assume::Preserve | Assume::Reach => {
                lines.push("inv(pre)".into());
                lines.push("typed(pre)".into());
                lines.push("typed(post)".into());
            }
            // No `typed(post)`. See the doc comment on the emitted
            // obligation -- this omission is the fix.
            Assume::Arith => {
                if self.two_state {
                    lines.push("inv(pre)".into());
                    lines.push("typed(pre)".into());
                } else {
                    lines.push("true".into());
                }
            }
        }
        for p in &self.params {
            if let RangeFacts::Integer { min, max, .. } = &p.facts {
                lines.push(format!("{min} <= {} <= {max}", p.name));
            }
        }
        if let Some(pre) = &self.pre_call {
            lines.push(pre.clone());
        }
        lines.push(self.post_call.clone());
        lines
            .iter()
            .map(|l| format!("        {l},\n"))
            .collect::<String>()
    }
}

fn spec_type(facts: &RangeFacts) -> &'static str {
    match facts {
        RangeFacts::Bool => "bool",
        _ => "int",
    }
}

/// Translate one premise's contract, or refuse it naming the clause.
fn contract_of(p: &Premise, property: &Property) -> Result<Contract, Untranslatable> {
    let two_state = p.role != PremiseRole::Constructor;
    let name = sym(&p.item);
    let params = p.parameters.clone();
    let decl_params: String = params
        .iter()
        .map(|q| format!(", {}: {}", q.name, spec_type(&q.facts)))
        .collect();
    let call_params: String = params
        .iter()
        .map(|q| format!(", {}", q.name))
        .collect::<String>();

    let mut range = Vec::new();
    let mut post_clauses = Vec::new();
    for clause in &p.ensures {
        post_clauses.push(translate_clause(clause, property, &params)?);
        range.extend(range_checks(clause, property, &params)?);
    }
    let mut pre_clauses = Vec::new();
    for clause in &p.requires {
        pre_clauses.push(translate(
            clause,
            property,
            &environment(property, &params),
            Bare::Pre,
        )?);
        // A precondition's arithmetic is the caller's to keep in range, and
        // the caller is outside this theorem. Recorded as untranslated
        // rather than silently checked or silently skipped.
        range.extend(range_checks(clause, property, &params)?);
    }

    let state_decl = if two_state {
        "pre: S, post: S"
    } else {
        "post: S"
    };
    let state_call = if two_state { "pre, post" } else { "post" };
    let mut declarations = String::new();
    declarations.push_str(&format!(
        "/// The contract of `{}`, exactly as it was proved.\n\
         pub open spec fn {name}_post({state_decl}{decl_params}) -> bool {{\n    true\n",
        p.item
    ));
    for c in &post_clauses {
        declarations.push_str(&format!("    && ({c})\n"));
    }
    declarations.push_str("}\n\n");

    let pre_call = if pre_clauses.is_empty() {
        None
    } else {
        declarations.push_str(&format!(
            "/// What `{}` requires of its caller.\n\
             pub open spec fn {name}_pre(pre: S{decl_params}) -> bool {{\n    true\n",
            p.item
        ));
        for c in &pre_clauses {
            declarations.push_str(&format!("    && ({c})\n"));
        }
        declarations.push_str("}\n\n");
        Some(format!("{name}_pre(pre{call_params})"))
    };

    Ok(Contract {
        declarations,
        two_state,
        params,
        pre_call,
        post_call: format!("{name}_post({state_call}{call_params})"),
        range,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{Domain, Inventory, Observer, Premise, Provenance, plan};

    fn obs(name: &str) -> Observer {
        Observer {
            name: name.into(),
            facts: RangeFacts::of_rust_type("u32", Some(64)),
            reads_are_pure: true,
        }
    }

    fn property() -> Property {
        Property {
            name: "never over capacity".into(),
            invariant: "available <= capacity".into(),
            state_type: "TokenBucket".into(),
            observers: vec![obs("available"), obs("capacity")],
        }
    }

    fn u32_param(name: &str) -> Parameter {
        Parameter {
            name: name.into(),
            facts: RangeFacts::of_rust_type("u32", Some(64)),
        }
    }

    fn bool_param(name: &str) -> Parameter {
        Parameter {
            name: name.into(),
            facts: RangeFacts::Bool,
        }
    }

    /// The parameter set the translation tests use: everything the bucket's
    /// contracts name.
    fn params() -> Vec<Parameter> {
        vec![u32_param("tokens"), u32_param("cap"), bool_param("ok")]
    }

    fn premise(
        item: &str,
        role: PremiseRole,
        parameters: Vec<Parameter>,
        ensures: &[&str],
    ) -> Premise {
        Premise {
            item: item.into(),
            role,
            source_fingerprint: "src1".into(),
            contract_fingerprint: "con1".into(),
            config: "cfg1".into(),
            domain: Domain::Unrestricted,
            parameters,
            requires: vec![],
            ensures: ensures.iter().map(|s| s.to_string()).collect(),
            frame: vec![],
            assumptions: vec![],
            provenance: Provenance::Proved {
                engine: "verus".into(),
                version: "0.2026.08.23.fbbbbcf".into(),
                artifact: "a1".into(),
            },
        }
    }

    fn bucket_premises() -> Vec<Premise> {
        vec![
            premise(
                "TokenBucket::new",
                PremiseRole::Constructor,
                vec![u32_param("cap")],
                &["available == cap", "capacity == cap"],
            ),
            premise(
                "TokenBucket::try_take",
                PremiseRole::Transition,
                vec![u32_param("tokens"), bool_param("ok")],
                &[
                    "ok == (old(available) >= tokens)",
                    "ok ==> available == old(available) - tokens",
                    "!ok ==> available == old(available)",
                    "capacity == old(capacity)",
                ],
            ),
            // Faithful to the source, which refills with `saturating_add`.
            // Written with a plain `+` this contract is not overflow-free
            // and the range obligation says so -- which is the check
            // working, not the fixture being awkward.
            premise(
                "TokenBucket::refill",
                PremiseRole::Transition,
                vec![u32_param("tokens")],
                &[
                    "available == min(saturating_add(old(available), tokens), old(capacity))",
                    "capacity == old(capacity)",
                ],
            ),
        ]
    }

    fn bucket_inventory() -> Inventory {
        Inventory {
            constructors: vec!["TokenBucket::new".into()],
            mutators: vec!["TokenBucket::try_take".into(), "TokenBucket::refill".into()],
            escapes: vec![],
            unclassified: vec![],
        }
    }

    pub(super) fn bucket_plan() -> (crate::compose::Plan, Vec<Premise>) {
        let premises = bucket_premises();
        let p = plan(
            &property(),
            &bucket_inventory(),
            &premises,
            &std::collections::BTreeMap::new(),
        );
        (p, premises)
    }

    // ---- the accepted subset ----

    /// Subtraction on an unsigned reading can go below zero, and that is
    /// exactly the case that made the first encoding unsound. The check
    /// must be generated, naming the operation.
    #[test]
    fn unsigned_subtraction_generates_a_lower_bound_check() {
        let checks = range_checks(
            "available == old(available) - tokens",
            &property(),
            &params(),
        )
        .unwrap();
        assert!(
            checks
                .iter()
                .any(|c| c.operation.contains('-') && c.condition.contains(">=")),
            "u32 subtraction must owe a non-negative check: {checks:?}"
        );
    }

    /// Addition can exceed the type's maximum, and the ceiling comes from
    /// the operands' declared type -- not from whatever reading happens to
    /// be widest.
    #[test]
    fn addition_generates_an_upper_bound_check_at_the_operands_declared_width() {
        let checks = range_checks(
            "available == old(available) + tokens",
            &property(),
            &params(),
        )
        .unwrap();
        assert!(
            checks.iter().any(|c| c.condition.contains("4294967295")),
            "u32 addition must owe an upper-bound check at u32::MAX: {checks:?}"
        );
    }

    /// **Short-circuiting is not a detail here.** A clause of the form
    /// `guard ==> available == old(available) - tokens` only asserts
    /// anything when the guard holds, so the subtraction only has to be in
    /// range when the guard holds. An unguarded obligation would be
    /// unprovable for a contract that is perfectly safe.
    #[test]
    fn an_operation_under_a_guard_owes_its_range_check_only_under_that_guard() {
        let checks = range_checks(
            "ok ==> available == old(available) - tokens",
            &property(),
            &params(),
        )
        .unwrap();
        let guarded = checks
            .iter()
            .find(|c| c.operation.contains('-'))
            .expect("the subtraction owes a check");
        assert_eq!(
            guarded.guard.as_deref(),
            Some("ok"),
            "the check must be conditioned on the guard: {guarded:?}"
        );
    }

    /// Two different widths in one operation means the normalised contract
    /// is not well typed, and picking one of them would silently widen or
    /// narrow the program.
    #[test]
    fn mixing_two_widths_in_one_operation_is_refused_rather_than_guessed() {
        let mut ps = params();
        ps.push(Parameter {
            name: "big".into(),
            facts: RangeFacts::of_rust_type("u64", Some(64)),
        });
        let err = range_checks("available == old(available) + big", &property(), &ps)
            .expect_err("u32 + u64 is not a Rust expression that type checks");
        assert!(err.reason.contains("width"), "{err:?}");
    }

    /// Arithmetic on a name with no known width has no ceiling to check
    /// against, so it is refused rather than checked against a guess.
    #[test]
    fn arithmetic_on_names_with_no_known_width_is_refused_rather_than_guessed() {
        assert!(range_checks("available == 1 + 2", &property(), &params()).is_err());
    }

    /// A clause with no arithmetic owes nothing, or every comparison would
    /// drag in a check nobody needs.
    #[test]
    fn a_clause_without_arithmetic_owes_no_range_check() {
        assert!(
            range_checks("available <= capacity", &property(), &params())
                .unwrap()
                .is_empty()
        );
        assert!(
            range_checks("ok == (old(available) >= tokens)", &property(), &params())
                .unwrap()
                .is_empty()
        );
    }

    /// An expression shape this adapter was not built for is refused by
    /// name. Dropping it silently would leave the rest looking complete.
    #[test]
    fn an_untranslatable_clause_is_refused_naming_itself() {
        let bad = translate_clause(
            "available == compute(self.inner, 3)",
            &property(),
            &params(),
        );
        let err = bad.expect_err("a call to an unknown function is not translatable");
        assert!(err.reason.contains("compute"), "{err:?}");
        assert_eq!(err.clause, "available == compute(self.inner, 3)");
    }

    /// A clause that does not parse as a Rust expression is refused, not
    /// guessed at.
    #[test]
    fn an_unparseable_clause_is_refused_rather_than_guessed() {
        assert!(translate_clause("available ==== capacity", &property(), &params()).is_err());
    }

    /// The supported shapes translate -- including the two saturating
    /// helpers the real contracts use, which are a fixed two-entry table
    /// and not the start of a general translator.
    #[test]
    fn the_supported_expression_shapes_translate() {
        for clause in [
            "available <= capacity",
            "available == old(available) - tokens",
            "ok ==> available == old(available)",
            "!ok ==> available == old(available)",
            "capacity == old(capacity)",
            "available == min(old(available) + tokens, old(capacity))",
            "available == max(old(available) - tokens, 0)",
        ] {
            assert!(
                translate_clause(clause, &property(), &params()).is_ok(),
                "should translate: {clause}"
            );
        }
    }

    /// `old(x)` must become the pre-state reading, never the post-state
    /// one, or every frame fact silently holds.
    #[test]
    fn old_becomes_the_pre_state_and_a_bare_name_the_post_state() {
        let out = translate_clause("available == old(available)", &property(), &params()).unwrap();
        assert!(out.contains("post.available"), "{out}");
        assert!(out.contains("pre.available"), "{out}");
    }

    // ---- the encoding ----

    /// **The invariant test.** Every obligation the planner asked for is
    /// either written into the source under a name, or reported as omitted
    /// with a reason. An obligation that is neither is one the solver was
    /// never asked about while the report counts it as handled.
    #[test]
    fn every_planned_obligation_is_either_emitted_or_reported_as_omitted() {
        let (p, premises) = bucket_plan();
        assert!(!p.obligations.is_empty(), "the fixture must plan something");
        let e = encode(&p, &premises);
        for ob in &p.obligations {
            let name = obligation_name(&ob.kind);
            let emitted = e.emitted.contains(&name);
            let omitted = e.omitted.iter().any(|(n, _)| *n == name);
            let trivial = e.trivial.iter().any(|(n, _)| *n == name);
            let landed = [emitted, omitted, trivial].iter().filter(|b| **b).count();
            assert_eq!(
                landed, 1,
                "{name} landed in {landed} of the three buckets: every obligation must be \
                 written into the source, reported as omitted, or reported as having nothing \
                 to check -- exactly one"
            );
            if emitted {
                let inverted = e.inverted.contains(&name);
                let file = if inverted { &e.probe } else { &e.source };
                assert!(
                    file.contains(&name),
                    "{name} is reported as emitted but is in neither file"
                );
                assert!(
                    !if inverted { &e.source } else { &e.probe }.contains(&name),
                    "{name} appears in both files, so a reader counting errors cannot tell \
                     which way it reads"
                );
            }
        }
    }

    /// A clause the adapter will not translate must take its obligation
    /// with it. Encoding the rest would ask the solver a smaller question
    /// and report the answer to the larger one.
    #[test]
    fn a_refused_clause_omits_its_obligation_instead_of_encoding_a_weaker_one() {
        let mut premises = bucket_premises();
        premises
            .iter_mut()
            .find(|p| p.item.ends_with("refill"))
            .unwrap()
            .ensures
            .push("available == wildly(old(capacity))".into());
        let p = plan(
            &property(),
            &bucket_inventory(),
            &premises,
            &std::collections::BTreeMap::new(),
        );
        let e = encode(&p, &premises);
        assert!(
            e.refused.iter().any(|u| u.item.ends_with("refill")),
            "the untranslatable clause must be named: {:?}",
            e.refused
        );
        assert!(
            e.omitted
                .iter()
                .any(|(n, _)| n.contains("preserve") && n.contains("refill")),
            "refill's preservation must be omitted, not encoded without the clause: {:?}",
            e.omitted
        );
        assert!(
            !e.source.contains("ob_preserve_TokenBucket_refill"),
            "the omitted obligation must not appear in the source at all"
        );
    }

    /// Preservation is the induction step: assume the invariant of the
    /// state before, prove it of the state after.
    #[test]
    fn preservation_assumes_the_invariant_before_and_proves_it_after() {
        let (p, premises) = bucket_plan();
        let body = obligation_text(
            &encode(&p, &premises).source,
            "ob_preserve_TokenBucket_refill",
        );
        let (before, after) = body.split_once("ensures").expect("has an ensures");
        assert!(before.contains("inv(pre)"), "{body}");
        assert!(after.contains("inv(post)"), "{body}");
    }

    /// The constructor obligation must not assume the invariant: assuming
    /// what is to be proved is the easiest way to make a solver agree.
    #[test]
    fn the_constructor_obligation_does_not_assume_the_invariant() {
        let (p, premises) = bucket_plan();
        let body = obligation_text(&encode(&p, &premises).source, "ob_init_TokenBucket_new");
        let requires = body.split("ensures").next().unwrap_or("");
        assert!(
            !requires.contains("inv("),
            "the invariant must not appear among the constructor's premises: {requires}"
        );
    }

    /// **The one obligation whose success is bad news.** A contradictory
    /// premise set discharges everything, so the probe asks the solver to
    /// derive a falsehood from the premises alone. It verifying means the
    /// contract is self-contradictory and every other result about that
    /// operation is worthless. The encoding has to say so where a reader
    /// will see it, because a green line that means failure is exactly the
    /// kind of thing that gets skimmed past.
    #[test]
    fn the_vacuity_probe_reads_backwards_and_the_source_says_so() {
        let (p, premises) = bucket_plan();
        let e = encode(&p, &premises);
        let body = obligation_text(&e.probe, "ob_reach_TokenBucket_try_take");
        assert!(
            body.contains("ensures false"),
            "the probe must ask for a falsehood: {body}"
        );
        assert!(
            e.probe.contains("VERIFYING THIS IS THE FAILURE"),
            "the inverted reading has to be stated where it is run"
        );
        assert!(
            !e.source.contains("ensures false"),
            "nothing that reads backwards may sit in the file whose errors are errors"
        );
        assert!(
            e.inverted
                .contains(&"ob_reach_TokenBucket_try_take".to_string()),
            "the inverted obligations must be listed for the reporter: {:?}",
            e.inverted
        );
    }

    /// A parameter's declared width has to reach the solver, or it ranges
    /// over values the program cannot pass.
    #[test]
    fn parameter_ranges_reach_the_encoding_as_premises() {
        let (p, premises) = bucket_plan();
        let body = obligation_text(
            &encode(&p, &premises).source,
            "ob_preserve_TokenBucket_try_take",
        );
        let requires = body.split("ensures").next().unwrap_or("");
        assert!(
            requires.contains("0 <= tokens <= 4294967295"),
            "the parameter's declared width is missing: {requires}"
        );
    }

    /// The arithmetic obligation has to name the operation it bounds, or
    /// a reader cannot tell which subtraction is unproved.
    #[test]
    fn the_arithmetic_obligation_bounds_the_operation_the_contract_performs() {
        let (p, premises) = bucket_plan();
        let e = encode(&p, &premises);
        let body = obligation_text(&e.source, "ob_arith_TokenBucket_try_take");
        assert!(
            body.contains("(ok) ==> (pre.available - tokens >= 0)"),
            "the subtraction's lower bound, conditioned on the guard that makes it safe, is \
             missing: {body}"
        );
    }

    /// The whole text of one obligation, from its name to the closing
    /// brace of its (empty) body.
    fn obligation_text(source: &str, name: &str) -> String {
        let at = source
            .find(&format!("proof fn {name}"))
            .unwrap_or_else(|| panic!("no obligation named {name} in:\n{source}"));
        let rest = &source[at..];
        // Every obligation ends with an empty body on its own line. Ending
        // at the first `\n}` instead ran past into the next obligation, so
        // these tests were reading text they had not asked for.
        let end = rest
            .find("\n{ }")
            .map(|i| i + 4)
            .unwrap_or_else(|| panic!("obligation {name} has no body terminator"));
        rest[..end].to_string()
    }
}
#[cfg(test)]
mod golden {
    /// The encoding the adapter produces for the token bucket, written out
    /// so `tests/spike/verus-component/compose/run.sh` can put it in front
    /// of a real solver. A doc claiming "it verifies" about a file nobody
    /// can run is the same defect as a green test nothing executes.
    #[test]
    fn the_emitted_encoding_matches_the_file_the_solver_is_run_against() {
        let (plan, premises) = super::tests::bucket_plan();
        let e = super::encode(&plan, &premises);
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/spike/verus-component/compose/"
        );
        for (file, emitted) in [
            ("generated_bucket.rs", &e.source),
            ("generated_bucket_probe.rs", &e.probe),
        ] {
            let path = format!("{dir}{file}");
            if std::env::var("PLY_BLESS").is_ok() {
                std::fs::write(&path, emitted).unwrap();
            }
            let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
            assert_eq!(
                &on_disk, emitted,
                "{file} is not what the adapter emits -- re-run with PLY_BLESS=1 and read the \
                 diff before accepting it"
            );
        }
    }
}
