//! Compact s-expression dump of the AST. Used by the syntax tests and by `ply check
//! --dump-ast`; it is deliberately lossy about spans and comments so tests read as grammar
//! assertions.

use crate::ast::*;
use std::fmt::Write;

pub fn dump_file(f: &File) -> String {
    let mut s = String::from("(file");
    for item in &f.items {
        s.push(' ');
        s.push_str(&dump_item(item));
    }
    s.push(')');
    s
}

pub fn dump_item(item: &Item) -> String {
    match &item.kind {
        ItemKind::Use(u) => format!("(use {})", u.name.name),
        ItemKind::Struct(d) => {
            let mut s = format!("(struct {} {}", d.name.name, generics(&d.generics));
            for f in &d.fields {
                let _ = write!(s, " (field {} {})", f.name.name, dump_type(&f.ty));
            }
            s.push(')');
            s
        }
        ItemKind::Enum(d) => {
            let mut s = format!("(enum {} {}", d.name.name, generics(&d.generics));
            for v in &d.variants {
                let _ = write!(s, " (variant {}", v.name.name);
                for t in &v.payload {
                    let _ = write!(s, " {}", dump_type(t));
                }
                s.push(')');
            }
            s.push(')');
            s
        }
        ItemKind::Fn(d) => dump_fn(d),
        ItemKind::Example(e) => {
            let args: Vec<String> = e.args.iter().map(dump_expr).collect();
            format!("(example {} ({}) {})", e.target.name, args.join(" "), dump_expr(&e.expected))
        }
        ItemKind::Rules(r) => {
            let mut s = format!("(rules {}", r.name.name);
            for rel in &r.rels {
                let _ = write!(s, " (rel {}", rel.name.name);
                for c in &rel.cols {
                    let _ = write!(s, " {}", dump_type(c));
                }
                s.push(')');
            }
            for rule in &r.rules {
                let _ = write!(s, " (rule {}", dump_atom(&rule.head));
                for b in &rule.body {
                    let _ = write!(s, " {}", dump_body_atom(b));
                }
                s.push(')');
            }
            s.push(')');
            s
        }
        ItemKind::Machine(m) => {
            let mut s = format!("(machine {}", m.name.name);
            for c in &m.state_chains {
                s.push_str(" (chain");
                for link in &c.links {
                    let names: Vec<&str> = link.iter().map(|i| i.name.as_str()).collect();
                    let _ = write!(s, " ({})", names.join(" "));
                }
                s.push(')');
            }
            for t in &m.transitions {
                let _ = write!(s, " (transition {} {}", t.from.name, t.to.name);
                if let Some(g) = &t.guard {
                    let _ = write!(s, " {}", dump_expr(g));
                }
                s.push(')');
            }
            for i in &m.invariants {
                let _ = write!(s, " (invariant {})", dump_expr(&i.expr));
            }
            s.push(')');
            s
        }
    }
}

fn dump_fn(d: &FnDecl) -> String {
    let mut s = format!("(fn {} {}", d.name.name, generics(&d.generics));
    for p in &d.params {
        let mode = match p.mode {
            ParamMode::Owned => "own",
            ParamMode::Ref => "ref",
            ParamMode::RefMut => "refmut",
        };
        let _ = write!(s, " (param {} {} {})", p.name.name, mode, dump_type(&p.ty));
    }
    let _ = write!(s, " -> {}", dump_type(&d.ret));
    if let Some(u) = &d.uses {
        s.push_str(" (uses");
        for c in &u.caps {
            let _ = write!(s, " {}", c.cap.as_str());
        }
        s.push(')');
    }
    for c in &d.contracts {
        match c {
            Contract::Requires { expr, .. } => {
                let _ = write!(s, " (requires {})", dump_expr(expr));
            }
            Contract::Ensures { binder, expr, .. } => {
                let _ = write!(s, " (ensures {} {})", binder.name, dump_expr(expr));
            }
            Contract::Decreases { expr, .. } => {
                let _ = write!(s, " (decreases {})", dump_expr(expr));
            }
        }
    }
    if let Some(v) = &d.verify {
        let _ = write!(s, " (verify {})", v.mode.describe());
    }
    let _ = write!(s, " {})", dump_block(&d.body));
    s
}

fn generics(g: &[Ident]) -> String {
    let names: Vec<&str> = g.iter().map(|i| i.name.as_str()).collect();
    format!("[{}]", names.join(" "))
}

pub fn dump_type(t: &Type) -> String {
    match &t.kind {
        TypeKind::Unit => "unit".to_string(),
        TypeKind::Error => "<error>".to_string(),
        TypeKind::Named { name, args } if args.is_empty() => name.name.clone(),
        TypeKind::Named { name, args } => {
            let a: Vec<String> = args.iter().map(dump_type).collect();
            format!("{}[{}]", name.name, a.join(", "))
        }
    }
}

pub fn dump_block(b: &Block) -> String {
    let mut s = String::from("(block");
    for st in &b.stmts {
        let _ = write!(s, " {}", dump_stmt(st));
    }
    if let Some(t) = &b.tail {
        let _ = write!(s, " {}", dump_expr(t));
    }
    s.push(')');
    s
}

pub fn dump_stmt(st: &Stmt) -> String {
    match &st.kind {
        StmtKind::Let { mutable, name, ty, init } => {
            let mut s = String::from("(let");
            if *mutable {
                s.push_str(" mut");
            }
            let _ = write!(s, " {}", name.name);
            if let Some(t) = ty {
                let _ = write!(s, " : {}", dump_type(t));
            }
            let _ = write!(s, " {})", dump_expr(init));
            s
        }
        StmtKind::Assign { target, value } => {
            format!("(assign {} {})", dump_expr(target), dump_expr(value))
        }
        StmtKind::While { cond, invariants, decreases, body } => {
            let mut s = format!("(while {}", dump_expr(cond));
            for i in invariants {
                let _ = write!(s, " (invariant {})", dump_expr(i));
            }
            if let Some(d) = decreases {
                let _ = write!(s, " (decreases {})", dump_expr(d));
            }
            let _ = write!(s, " {})", dump_block(body));
            s
        }
        StmtKind::For { var, iter, body } => {
            format!("(for {} {} {})", var.name, dump_expr(iter), dump_block(body))
        }
        StmtKind::Return(None) => "(return)".to_string(),
        StmtKind::Return(Some(e)) => format!("(return {})", dump_expr(e)),
        StmtKind::Expr(e) => format!("(expr {})", dump_expr(e)),
    }
}

pub fn dump_expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(v) => v.clone(),
        ExprKind::Str(v) => escape_string(v),
        ExprKind::Bool(true) => "true".to_string(),
        ExprKind::Bool(false) => "false".to_string(),
        ExprKind::Unit => "unit".to_string(),
        ExprKind::Path(i) => i.name.clone(),
        ExprKind::FieldRef(i) => format!("(fieldref {})", i.name),
        ExprKind::Binary { op, lhs, rhs } => {
            format!("({} {} {})", op.as_str(), dump_expr(lhs), dump_expr(rhs))
        }
        ExprKind::Unary { op, operand } => {
            let name = match op {
                UnOp::Not => "not",
                UnOp::Neg => "neg",
            };
            format!("({} {})", name, dump_expr(operand))
        }
        ExprKind::Call { callee, type_args, args } => {
            let mut s = format!("(call {}", dump_expr(callee));
            if !type_args.is_empty() {
                let t: Vec<String> = type_args.iter().map(dump_type).collect();
                let _ = write!(s, " [{}]", t.join(" "));
            }
            for a in args {
                let _ = write!(s, " {}", dump_expr(a));
            }
            s.push(')');
            s
        }
        ExprKind::MethodCall { receiver, method, type_args, args } => {
            let mut s = format!("(method {} {}", dump_expr(receiver), method.name);
            if !type_args.is_empty() {
                let t: Vec<String> = type_args.iter().map(dump_type).collect();
                let _ = write!(s, " [{}]", t.join(" "));
            }
            for a in args {
                let _ = write!(s, " {}", dump_expr(a));
            }
            s.push(')');
            s
        }
        ExprKind::Field { base, name } => format!("(field {} {})", dump_expr(base), name.name),
        ExprKind::Index { base, index } => format!("(index {} {})", dump_expr(base), dump_expr(index)),
        ExprKind::Block(b) => dump_block(b),
        ExprKind::If { cond, then, else_ } => {
            let mut s = format!("(if {} {}", dump_expr(cond), dump_block(then));
            if let Some(e) = else_ {
                let _ = write!(s, " {}", dump_expr(e));
            }
            s.push(')');
            s
        }
        ExprKind::Match { scrutinee, arms } => {
            let mut s = format!("(match {}", dump_expr(scrutinee));
            for a in arms {
                let _ = write!(s, " (arm {} {})", dump_pattern(&a.pattern), dump_expr(&a.body));
            }
            s.push(')');
            s
        }
        ExprKind::List(items) => {
            let mut s = String::from("(list");
            for i in items {
                let _ = write!(s, " {}", dump_expr(i));
            }
            s.push(')');
            s
        }
        ExprKind::StructLit { name, fields } => {
            let mut s = format!("(struct-lit {}", name.name);
            for f in fields {
                let _ = write!(s, " ({} {})", f.name.name, dump_expr(&f.value));
            }
            s.push(')');
            s
        }
        ExprKind::Query(q) => dump_query(q),
        ExprKind::Unresolved { id } => format!("(unresolved {id})"),
        ExprKind::Dontcare => "dontcare".to_string(),
        ExprKind::Error => "<error>".to_string(),
    }
}

fn dump_query(q: &QueryExpr) -> String {
    let mut s = String::from("(query");
    for f in &q.froms {
        let _ = write!(s, " (from {} {})", f.var.name, dump_expr(&f.source));
    }
    if let Some(w) = &q.filter {
        let _ = write!(s, " (where {})", dump_expr(w));
    }
    if let Some(g) = &q.group {
        let _ = write!(s, " (group {} {} {})", g.row.name, dump_expr(&g.key), g.binding.name);
    }
    let _ = write!(s, " (select {})", dump_expr(&q.select));
    if let Some(o) = &q.order {
        let dir = if o.descending { "desc" } else { "asc" };
        let _ = write!(s, " (order {} {})", dump_expr(&o.key), dir);
    }
    if let Some(h) = &q.hint {
        let _ = write!(s, " (hint {} {})", h.key.name, h.value.name);
    }
    s.push(')');
    s
}

pub fn dump_pattern(p: &Pattern) -> String {
    match &p.kind {
        PatternKind::Wildcard => "_".to_string(),
        PatternKind::Binding(i) => i.name.clone(),
        PatternKind::Int(v) => v.clone(),
        PatternKind::Str(v) => escape_string(v),
        PatternKind::Bool(true) => "true".to_string(),
        PatternKind::Bool(false) => "false".to_string(),
        PatternKind::Unit => "unit".to_string(),
        PatternKind::Variant { name, args } => {
            let mut s = format!("(variant {}", name.name);
            for a in args.iter().flatten() {
                let _ = write!(s, " {}", dump_pattern(a));
            }
            s.push(')');
            s
        }
        PatternKind::Struct { fields } => {
            let mut s = String::from("(pat-struct");
            for f in fields {
                match &f.pattern {
                    None => {
                        let _ = write!(s, " ({})", f.name.name);
                    }
                    Some(p) => {
                        let _ = write!(s, " ({} {})", f.name.name, dump_pattern(p));
                    }
                }
            }
            s.push(')');
            s
        }
        PatternKind::Error => "<error>".to_string(),
    }
}

fn dump_atom(a: &RuleAtom) -> String {
    let mut s = format!("(atom {}", a.name.name);
    for t in &a.terms {
        let _ = write!(s, " {}", dump_term(t));
    }
    s.push(')');
    s
}

fn dump_body_atom(b: &BodyAtom) -> String {
    match b {
        BodyAtom::Pred { negated: false, atom } => dump_atom(atom),
        BodyAtom::Pred { negated: true, atom } => format!("(not {})", dump_atom(atom)),
        BodyAtom::Cmp { lhs, op, rhs, .. } => {
            format!("(cmp {} {} {})", op.as_str(), dump_term(lhs), dump_term(rhs))
        }
    }
}

fn dump_term(t: &Term) -> String {
    match t {
        Term::Var(i) => i.name.clone(),
        Term::Int(v, _) => v.clone(),
        Term::Str(v, _) => escape_string(v),
        Term::Bool(b, _) => b.to_string(),
    }
}

/// Re-escape a decoded string value into Ply source form.
pub fn escape_string(v: &str) -> String {
    let mut s = String::with_capacity(v.len() + 2);
    s.push('"');
    for c in v.chars() {
        match c {
            '\n' => s.push_str("\\n"),
            '\t' => s.push_str("\\t"),
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(s, "\\u{{{:X}}}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}
