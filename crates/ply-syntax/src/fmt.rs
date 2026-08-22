//! The canonical formatter (§14). Zero options: the layout is a function of the tree and
//! the fixed width, so `fmt(fmt(x)) == fmt(x)` and every author of a Ply file produces the
//! same bytes.

use crate::ast::*;
use crate::doc::{self, Doc, concat, group, hardline, if_break, join, line, nest, nil, softline, text};
use crate::dump::escape_string;

const WIDTH: usize = 100;
const INDENT: usize = 4;

pub fn format_file(f: &File) -> String {
    let mut out = doc::render(&file_doc(f), WIDTH);
    while out.ends_with('\n') {
        out.pop();
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn file_doc(f: &File) -> Doc {
    let mut parts = Vec::new();
    let mut prev: Option<&Item> = None;
    for item in &f.items {
        if let Some(p) = prev {
            parts.push(hardline());
            if blank_between(p, item) {
                parts.push(hardline());
            }
        }
        parts.push(item_doc(item));
        prev = Some(item);
    }
    if !f.trailing_comments.is_empty() {
        if prev.is_some() {
            parts.push(hardline());
            parts.push(hardline());
        }
        parts.push(comments_doc(&f.trailing_comments));
    }
    concat(parts)
}

/// Items are separated by a blank line, except runs of the one-line item forms (`use`,
/// `example`), which stay packed unless the author separated them.
fn blank_between(prev: &Item, cur: &Item) -> bool {
    let packed = matches!(
        (&prev.kind, &cur.kind),
        (ItemKind::Use(_), ItemKind::Use(_)) | (ItemKind::Example(_), ItemKind::Example(_))
    );
    if packed { cur.comments.blank_before } else { true }
}

// ---------------------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------------------

/// Comments are emitted verbatim: the formatter lays out code, never comment text.
fn comment_doc(c: &Comment) -> Doc {
    text(c.text.clone())
}

fn comments_doc(cs: &[Comment]) -> Doc {
    let mut parts = Vec::new();
    for (i, c) in cs.iter().enumerate() {
        if i > 0 {
            parts.push(hardline());
            if c.blank_before {
                parts.push(hardline());
            }
        }
        parts.push(comment_doc(c));
    }
    concat(parts)
}

/// `leading comments` + node + ` // trailing`.
fn with_comments(c: &Comments, body: Doc) -> Doc {
    let mut parts = Vec::new();
    if !c.leading.is_empty() {
        parts.push(comments_doc(&c.leading));
        parts.push(hardline());
        if c.blank_after_leading {
            parts.push(hardline());
        }
    }
    parts.push(body);
    if let Some(t) = &c.trailing {
        parts.push(text(" "));
        parts.push(comment_doc(t));
    }
    concat(parts)
}

// ---------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------

fn item_doc(item: &Item) -> Doc {
    let body = match &item.kind {
        ItemKind::Use(u) => concat(vec![text("use "), text(u.name.name.clone()), text(";")]),
        ItemKind::Struct(d) => struct_doc(d),
        ItemKind::Enum(d) => enum_doc(d),
        ItemKind::Fn(d) => fn_doc(d),
        ItemKind::Example(d) => example_doc(d),
        ItemKind::Rules(d) => rules_doc(d),
        ItemKind::Machine(d) => machine_doc(d),
    };
    with_comments(&item.comments, body)
}

fn generics_doc(g: &[Ident]) -> Doc {
    if g.is_empty() {
        return nil();
    }
    let names: Vec<String> = g.iter().map(|i| i.name.clone()).collect();
    text(format!("[{}]", names.join(", ")))
}

/// A `{ ... }` list with one entry per line and a trailing comma.
fn braced_lines(header: Doc, entries: Vec<Doc>) -> Doc {
    if entries.is_empty() {
        return concat(vec![header, text(" {}")]);
    }
    let mut inner = Vec::new();
    for e in entries {
        inner.push(hardline());
        inner.push(e);
    }
    concat(vec![header, text(" {"), nest(INDENT, concat(inner)), hardline(), text("}")])
}

fn struct_doc(d: &StructDecl) -> Doc {
    let entries = d
        .fields
        .iter()
        .map(|f| {
            with_comments(
                &f.comments,
                concat(vec![text(f.name.name.clone()), text(": "), type_doc(&f.ty), text(",")]),
            )
        })
        .collect();
    braced_lines(
        concat(vec![text("struct "), text(d.name.name.clone()), generics_doc(&d.generics)]),
        entries,
    )
}

fn enum_doc(d: &EnumDecl) -> Doc {
    let entries = d
        .variants
        .iter()
        .map(|v| {
            let payload = if v.payload.is_empty() {
                nil()
            } else {
                let tys: Vec<Doc> = v.payload.iter().map(type_doc).collect();
                concat(vec![text("("), join(text(", "), tys), text(")")])
            };
            with_comments(
                &v.comments,
                concat(vec![text(v.name.name.clone()), payload, text(",")]),
            )
        })
        .collect();
    braced_lines(
        concat(vec![text("enum "), text(d.name.name.clone()), generics_doc(&d.generics)]),
        entries,
    )
}

fn fn_doc(d: &FnDecl) -> Doc {
    let params: Vec<Doc> = d
        .params
        .iter()
        .map(|p| {
            let mode = match p.mode {
                ParamMode::Owned => "",
                ParamMode::Ref => "&",
                ParamMode::RefMut => "&mut ",
            };
            with_comments(
                &p.comments,
                concat(vec![
                    text(p.name.name.clone()),
                    text(": "),
                    text(mode),
                    type_doc(&p.ty),
                ]),
            )
        })
        .collect();

    let sig = group(concat(vec![
        text("fn "),
        text(d.name.name.clone()),
        generics_doc(&d.generics),
        text("("),
        comma_list(params),
        text(")"),
        text(" -> "),
        type_doc(&d.ret),
    ]));

    let mut clauses: Vec<Doc> = Vec::new();
    if let Some(u) = &d.uses {
        clauses.push(uses_doc(u));
    }
    for c in &d.contracts {
        clauses.push(contract_doc(c));
    }
    if let Some(v) = &d.verify {
        clauses.push(concat(vec![text("verify "), text(v.mode.describe())]));
    }

    if clauses.is_empty() {
        concat(vec![sig, text(" "), block_doc(&d.body)])
    } else {
        let mut inner = Vec::new();
        for c in clauses {
            inner.push(hardline());
            inner.push(c);
        }
        concat(vec![sig, nest(INDENT, concat(inner)), hardline(), block_doc(&d.body)])
    }
}

fn uses_doc(u: &UsesClause) -> Doc {
    if u.caps.is_empty() {
        return text("uses {}");
    }
    let names: Vec<String> = u.caps.iter().map(|c| c.cap.as_str().to_string()).collect();
    text(format!("uses {{ {} }}", names.join(", ")))
}

fn contract_doc(c: &Contract) -> Doc {
    let body = match c {
        Contract::Requires { expr, .. } => concat(vec![text("requires "), expr_doc(expr, 0, false)]),
        Contract::Ensures { binder, expr, .. } => concat(vec![
            text("ensures |"),
            text(binder.name.clone()),
            text("| "),
            expr_doc(expr, 0, false),
        ]),
        Contract::Decreases { expr, .. } => {
            concat(vec![text("decreases "), expr_doc(expr, 0, false)])
        }
    };
    with_comments(c.comments(), body)
}

fn example_doc(d: &ExampleDecl) -> Doc {
    let args: Vec<Doc> = d.args.iter().map(|a| expr_doc(a, 0, false)).collect();
    group(concat(vec![
        text("example "),
        text(d.target.name.clone()),
        text("("),
        comma_list(args),
        text(")"),
        text(" == "),
        expr_doc(&d.expected, 0, false),
        text(";"),
    ]))
}

/// `a, b, c` flat; one per line with a trailing comma when broken.
fn comma_list(items: Vec<Doc>) -> Doc {
    if items.is_empty() {
        return nil();
    }
    concat(vec![
        nest(
            INDENT,
            concat(vec![
                softline(),
                join(concat(vec![text(","), line()]), items),
                if_break(",", ""),
            ]),
        ),
        softline(),
    ])
}

// ---------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------

fn type_doc(t: &Type) -> Doc {
    match &t.kind {
        TypeKind::Unit => text("()"),
        TypeKind::Error => text("()"),
        TypeKind::Named { name, args } if args.is_empty() => text(name.name.clone()),
        TypeKind::Named { name, args } => {
            let a: Vec<Doc> = args.iter().map(type_doc).collect();
            concat(vec![text(name.name.clone()), text("["), join(text(", "), a), text("]")])
        }
    }
}

// ---------------------------------------------------------------------------------------
// Blocks and statements
// ---------------------------------------------------------------------------------------

fn block_doc(b: &Block) -> Doc {
    if b.stmts.is_empty() && b.tail.is_none() && b.trailing_comments.is_empty() {
        return text("{}");
    }
    let mut inner = Vec::new();
    let mut first = true;
    for s in &b.stmts {
        inner.push(hardline());
        if !first && s.comments.blank_before {
            inner.push(hardline());
        }
        inner.push(stmt_doc(s));
        first = false;
    }
    if let Some(t) = &b.tail {
        inner.push(hardline());
        inner.push(expr_doc(t, 0, false));
        first = false;
    }
    if !b.trailing_comments.is_empty() {
        inner.push(hardline());
        if !first && b.trailing_comments[0].blank_before {
            inner.push(hardline());
        }
        inner.push(comments_doc(&b.trailing_comments));
    }
    concat(vec![text("{"), nest(INDENT, concat(inner)), hardline(), text("}")])
}

fn stmt_doc(s: &Stmt) -> Doc {
    let body = match &s.kind {
        StmtKind::Let { mutable, name, ty, init } => {
            let mut parts = vec![text("let ")];
            if *mutable {
                parts.push(text("mut "));
            }
            parts.push(text(name.name.clone()));
            if let Some(t) = ty {
                parts.push(text(": "));
                parts.push(type_doc(t));
            }
            parts.push(text(" = "));
            parts.push(expr_doc(init, 0, false));
            parts.push(text(";"));
            group(concat(parts))
        }
        StmtKind::Assign { target, value } => group(concat(vec![
            expr_doc(target, 0, false),
            text(" = "),
            expr_doc(value, 0, false),
            text(";"),
        ])),
        StmtKind::While { cond, invariants, decreases, body } => {
            let head = concat(vec![text("while "), expr_doc_cond(cond)]);
            let mut annots = Vec::new();
            for i in invariants {
                annots.push(hardline());
                annots.push(concat(vec![text("invariant "), expr_doc(i, 0, false)]));
            }
            if let Some(d) = decreases {
                annots.push(hardline());
                annots.push(concat(vec![text("decreases "), expr_doc(d, 0, false)]));
            }
            if annots.is_empty() {
                concat(vec![head, text(" "), block_doc(body)])
            } else {
                concat(vec![
                    head,
                    nest(INDENT, concat(annots)),
                    hardline(),
                    block_doc(body),
                ])
            }
        }
        StmtKind::For { var, iter, body } => concat(vec![
            text("for "),
            text(var.name.clone()),
            text(" in "),
            expr_doc_cond(iter),
            text(" "),
            block_doc(body),
        ]),
        StmtKind::Return(None) => text("return;"),
        StmtKind::Return(Some(e)) => {
            group(concat(vec![text("return "), expr_doc(e, 0, false), text(";")]))
        }
        StmtKind::Expr(e) if is_block_like(e) => expr_doc(e, 0, false),
        StmtKind::Expr(e) => group(concat(vec![expr_doc(e, 0, false), text(";")])),
    };
    with_comments(&s.comments, body)
}

fn is_block_like(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Block(_) | ExprKind::If { .. } | ExprKind::Match { .. })
}

// ---------------------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------------------

/// Binding power used to decide parentheses. Postfix and atoms sit above every operator;
/// block-like expressions sit below, so they are parenthesised whenever used as an operand.
const P_ATOM: u8 = 8;
const P_UNARY: u8 = 7;
const P_BLOCK: u8 = 0;

fn prec_of(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Binary { op, .. } => op.precedence(),
        ExprKind::Unary { .. } => P_UNARY,
        ExprKind::Block(_) | ExprKind::If { .. } | ExprKind::Match { .. } => P_BLOCK,
        _ => P_ATOM,
    }
}

/// `min_prec` is the binding power required by the context; `right` marks the right operand
/// of a left-associative operator, which needs parentheses at equal precedence.
fn expr_doc(e: &Expr, min_prec: u8, right: bool) -> Doc {
    expr_doc_in(e, min_prec, right, false)
}

/// Condition position: `if`, `while`, `for ... in` and the `match` scrutinee, where the
/// parser does not accept a struct literal because `{` starts the body. The flag follows
/// exactly the parser's rule — it propagates through operators and projections, and is
/// cleared inside anything that re-opens a bracket (§5.2).
fn expr_doc_cond(e: &Expr) -> Doc {
    expr_doc_in(e, 0, false, true)
}

fn expr_doc_in(e: &Expr, min_prec: u8, right: bool, cond: bool) -> Doc {
    let p = prec_of(e);
    let needs_parens = p < min_prec
        || (right && p == min_prec && p != P_ATOM)
        || (cond && starts_with_struct_lit(e));
    let inner = expr_body(e, cond && !needs_parens);
    if needs_parens {
        group(concat(vec![text("("), inner, text(")")]))
    } else {
        inner
    }
}

/// Would the printed form put a `{` where the parser is looking for a block?
fn starts_with_struct_lit(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::StructLit { .. })
}

fn expr_body(e: &Expr, cond: bool) -> Doc {
    match &e.kind {
        ExprKind::Int(v) => text(v.clone()),
        ExprKind::Str(v) => text(escape_string(v)),
        ExprKind::Bool(b) => text(if *b { "true" } else { "false" }),
        ExprKind::Unit => text("()"),
        ExprKind::Path(i) => text(i.name.clone()),
        ExprKind::FieldRef(i) => concat(vec![text("."), text(i.name.clone())]),
        ExprKind::Dontcare => text("dontcare"),
        ExprKind::Unresolved { id } => text(format!("unresolved #{id}")),
        ExprKind::Error => text("()"),
        ExprKind::Unary { op, operand } => {
            let sep = if op == &UnOp::Not { " " } else { "" };
            concat(vec![text(op.as_str()), text(sep), expr_doc_in(operand, P_UNARY, false, cond)])
        }
        ExprKind::Binary { op, lhs, rhs } if op.is_comparison() => {
            // Comparisons do not chain, so an operand of equal precedence on *either* side
            // needs parentheses to survive a reparse.
            let p = op.precedence();
            group(concat(vec![
                expr_doc_in(lhs, p + 1, false, cond),
                nest(
                    INDENT,
                    concat(vec![
                        line(),
                        text(op.as_str()),
                        text(" "),
                        expr_doc_in(rhs, p + 1, false, false),
                    ]),
                ),
            ]))
        }
        ExprKind::Binary { op, .. } => {
            // Flatten the left spine of same-precedence operators so a long chain breaks
            // before every operator instead of nesting one level deeper each time.
            let p = op.precedence();
            let mut spine: Vec<(BinOp, &Expr)> = Vec::new();
            let mut head = e;
            while let ExprKind::Binary { op, lhs, rhs } = &head.kind {
                if op.precedence() != p {
                    break;
                }
                spine.push((*op, rhs));
                head = lhs;
            }
            spine.reverse();
            let mut tail = Vec::new();
            for (op, rhs) in spine {
                tail.push(line());
                tail.push(text(op.as_str()));
                tail.push(text(" "));
                tail.push(expr_doc_in(rhs, p, true, false));
            }
            group(concat(vec![expr_doc_in(head, p, false, cond), nest(INDENT, concat(tail))]))
        }
        ExprKind::Call { callee, type_args, args } => {
            let a: Vec<Doc> = args.iter().map(|x| expr_doc(x, 0, false)).collect();
            group(concat(vec![
                expr_doc_in(callee, P_ATOM, false, cond),
                type_args_doc(type_args),
                text("("),
                comma_list(a),
                text(")"),
            ]))
        }
        ExprKind::MethodCall { receiver, method, type_args, args } => {
            let a: Vec<Doc> = args.iter().map(|x| expr_doc(x, 0, false)).collect();
            group(concat(vec![
                expr_doc_in(receiver, P_ATOM, false, cond),
                text("."),
                text(method.name.clone()),
                type_args_doc(type_args),
                text("("),
                comma_list(a),
                text(")"),
            ]))
        }
        ExprKind::Field { base, name } => concat(vec![
            expr_doc_in(base, P_ATOM, false, cond),
            text("."),
            text(name.name.clone()),
        ]),
        ExprKind::Index { base, index } => concat(vec![
            expr_doc_in(base, P_ATOM, false, cond),
            text("["),
            expr_doc(index, 0, false),
            text("]"),
        ]),
        ExprKind::List(items) => {
            let a: Vec<Doc> = items.iter().map(|x| expr_doc(x, 0, false)).collect();
            group(concat(vec![text("["), comma_list(a), text("]")]))
        }
        ExprKind::StructLit { name, fields } => {
            if fields.is_empty() {
                return concat(vec![text(name.name.clone()), text(" {}")]);
            }
            let f: Vec<Doc> = fields
                .iter()
                .map(|f| {
                    with_comments(
                        &f.comments,
                        concat(vec![
                            text(f.name.name.clone()),
                            text(": "),
                            expr_doc(&f.value, 0, false),
                        ]),
                    )
                })
                .collect();
            group(concat(vec![
                text(name.name.clone()),
                text(" {"),
                nest(
                    INDENT,
                    concat(vec![
                        line(),
                        join(concat(vec![text(","), line()]), f),
                        if_break(",", ""),
                    ]),
                ),
                line(),
                text("}"),
            ]))
        }
        ExprKind::Block(b) => block_doc(b),
        ExprKind::If { cond, then, else_ } => {
            let mut parts = vec![
                text("if "),
                expr_doc_cond(cond),
                text(" "),
                block_doc(then),
            ];
            if let Some(e) = else_ {
                parts.push(text(" else "));
                parts.push(expr_body(e, false));
            }
            concat(parts)
        }
        ExprKind::Match { scrutinee, arms } => {
            let entries: Vec<Doc> = arms
                .iter()
                .map(|a| {
                    with_comments(
                        &a.comments,
                        concat(vec![
                            pattern_doc(&a.pattern),
                            text(" => "),
                            expr_doc(&a.body, 0, false),
                            text(","),
                        ]),
                    )
                })
                .collect();
            braced_lines(concat(vec![text("match "), expr_doc_cond(scrutinee)]), entries)
        }
        ExprKind::Query(q) => query_doc(q),
    }
}

fn type_args_doc(args: &[Type]) -> Doc {
    if args.is_empty() {
        return nil();
    }
    let a: Vec<Doc> = args.iter().map(type_doc).collect();
    concat(vec![text("["), join(text(", "), a), text("]")])
}

fn pattern_doc(p: &Pattern) -> Doc {
    match &p.kind {
        PatternKind::Wildcard => text("_"),
        PatternKind::Binding(i) => text(i.name.clone()),
        PatternKind::Int(v) => text(v.clone()),
        PatternKind::Str(v) => text(escape_string(v)),
        PatternKind::Bool(b) => text(if *b { "true" } else { "false" }),
        PatternKind::Unit => text("()"),
        PatternKind::Error => text("_"),
        PatternKind::Variant { name, args } => match args {
            None => text(name.name.clone()),
            Some(sub) => {
                let s: Vec<Doc> = sub.iter().map(pattern_doc).collect();
                concat(vec![text(name.name.clone()), text("("), join(text(", "), s), text(")")])
            }
        },
        PatternKind::Struct { fields } => {
            let f: Vec<Doc> = fields
                .iter()
                .map(|f| match &f.pattern {
                    None => text(f.name.name.clone()),
                    Some(p) => {
                        concat(vec![text(f.name.name.clone()), text(": "), pattern_doc(p)])
                    }
                })
                .collect();
            concat(vec![text("{ "), join(text(", "), f), text(" }")])
        }
    }
}

// ---------------------------------------------------------------------------------------
// query / rules / machine
// ---------------------------------------------------------------------------------------

fn query_doc(q: &QueryExpr) -> Doc {
    let mut clauses: Vec<Doc> = Vec::new();
    for (i, f) in q.froms.iter().enumerate() {
        let comma = if i + 1 < q.froms.len() { text(",") } else { nil() };
        clauses.push(with_comments(
            &f.comments,
            concat(vec![
                text("from "),
                text(f.var.name.clone()),
                text(" in "),
                expr_doc(&f.source, 0, false),
                comma,
            ]),
        ));
    }
    if let Some(w) = &q.filter {
        clauses.push(concat(vec![text("where "), expr_doc(w, 0, false)]));
    }
    if let Some(g) = &q.group {
        clauses.push(concat(vec![
            text("group "),
            text(g.row.name.clone()),
            text(" by "),
            expr_doc(&g.key, 0, false),
            text(" into "),
            text(g.binding.name.clone()),
        ]));
    }
    clauses.push(concat(vec![text("select "), expr_doc(&q.select, 0, false)]));
    if let Some(o) = &q.order {
        let dir = if o.descending { " desc" } else { "" };
        clauses.push(concat(vec![text("order by "), expr_doc(&o.key, 0, false), text(dir)]));
    }
    if let Some(h) = &q.hint {
        clauses.push(text(format!("hint({}: {})", h.key.name, h.value.name)));
    }
    group(concat(vec![
        text("query {"),
        nest(INDENT, concat(vec![line(), join(line(), clauses)])),
        line(),
        text("}"),
    ]))
}

fn rules_doc(d: &RulesDecl) -> Doc {
    let mut entries: Vec<Doc> = Vec::new();
    for r in &d.rels {
        let cols: Vec<Doc> = r.cols.iter().map(type_doc).collect();
        entries.push(with_comments(
            &r.comments,
            concat(vec![
                text("rel "),
                text(r.name.name.clone()),
                text("("),
                join(text(", "), cols),
                text(");"),
            ]),
        ));
    }
    for r in &d.rules {
        let body: Vec<Doc> = r.body.iter().map(body_atom_doc).collect();
        entries.push(with_comments(
            &r.comments,
            group(concat(vec![
                atom_doc(&r.head),
                text(" :-"),
                nest(
                    INDENT,
                    concat(vec![line(), join(concat(vec![text(","), line()]), body)]),
                ),
                text(";"),
            ])),
        ));
    }
    braced_lines(concat(vec![text("rules "), text(d.name.name.clone())]), entries)
}

fn atom_doc(a: &RuleAtom) -> Doc {
    let terms: Vec<Doc> = a.terms.iter().map(term_doc).collect();
    concat(vec![text(a.name.name.clone()), text("("), join(text(", "), terms), text(")")])
}

fn body_atom_doc(b: &BodyAtom) -> Doc {
    match b {
        BodyAtom::Pred { negated: false, atom } => atom_doc(atom),
        BodyAtom::Pred { negated: true, atom } => concat(vec![text("not "), atom_doc(atom)]),
        BodyAtom::Cmp { lhs, op, rhs, .. } => concat(vec![
            term_doc(lhs),
            text(" "),
            text(op.as_str()),
            text(" "),
            term_doc(rhs),
        ]),
    }
}

fn term_doc(t: &Term) -> Doc {
    match t {
        Term::Var(i) => text(i.name.clone()),
        Term::Int(v, _) => text(v.clone()),
        Term::Str(v, _) => text(escape_string(v)),
        Term::Bool(b, _) => text(b.to_string()),
    }
}

fn machine_doc(d: &MachineDecl) -> Doc {
    let mut entries: Vec<Doc> = Vec::new();
    for (i, c) in d.state_chains.iter().enumerate() {
        let links: Vec<Doc> = c
            .links
            .iter()
            .map(|alts| {
                let names: Vec<String> = alts.iter().map(|a| a.name.clone()).collect();
                text(names.join(" | "))
            })
            .collect();
        let prefix = if i == 0 { text("states ") } else { nil() };
        entries.push(with_comments(
            &c.comments,
            concat(vec![prefix, join(text(" -> "), links), text(";")]),
        ));
    }
    for t in &d.transitions {
        let guard = match &t.guard {
            Some(g) => concat(vec![text(" when "), expr_doc(g, 0, false)]),
            None => nil(),
        };
        entries.push(with_comments(
            &t.comments,
            concat(vec![
                text(t.from.name.clone()),
                text(" -> "),
                text(t.to.name.clone()),
                guard,
                text(";"),
            ]),
        ));
    }
    for i in &d.invariants {
        entries.push(with_comments(
            &i.comments,
            concat(vec![text("invariant: "), expr_doc(&i.expr, 0, false), text(";")]),
        ));
    }
    braced_lines(concat(vec![text("machine "), text(d.name.name.clone())]), entries)
}

/// Print a single expression (used by the round-trip property tests and by `ply explain`).
pub fn format_expr(e: &Expr) -> String {
    doc::render(&expr_doc(e, 0, false), WIDTH)
}

/// Print an expression as it must appear in condition position.
pub fn format_cond_expr(e: &Expr) -> String {
    doc::render(&expr_doc_cond(e), WIDTH)
}
