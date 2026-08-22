//! `E0101`: naming conventions are part of the language, not a style guide (§5.1).
//!
//! Types are `UpperCamel`, everything else is `snake_case`. Every violation carries the
//! converted name as a mechanical fix, so an agent can apply it without guessing.

use crate::ast::*;
use ply_diag::{Code, Diagnostic, Diagnostics, Fix};

#[derive(Copy, Clone, PartialEq, Eq)]
enum Case {
    Type,
    Value,
}

pub fn check_file(f: &File, out: &mut Diagnostics) {
    let mut cx = Cx { out };
    for item in &f.items {
        cx.item(item);
    }
}

struct Cx<'a> {
    out: &'a mut Diagnostics,
}

impl Cx<'_> {
    fn ident(&mut self, i: &Ident, case: Case) {
        if i.name == "_" || i.name.is_empty() {
            return;
        }
        let ok = match case {
            Case::Type => is_upper_camel(&i.name),
            Case::Value => is_snake(&i.name),
        };
        if ok {
            return;
        }
        let (what, want, suggestion) = match case {
            Case::Type => ("types", "UpperCamel", to_upper_camel(&i.name)),
            Case::Value => ("values", "snake_case", to_snake(&i.name)),
        };
        let mut d = Diagnostic::new(
            Code::E0101,
            i.span,
            format!("`{}` is not {want}; {what} in Ply are written {want}", i.name),
        );
        if suggestion != i.name && !suggestion.is_empty() {
            d = d.with_fix(Fix::replace(format!("rename to `{suggestion}`"), i.span, suggestion));
        }
        self.out.push(d);
    }

    fn item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Use(u) => self.ident(&u.name, Case::Value),
            ItemKind::Struct(d) => {
                self.ident(&d.name, Case::Type);
                for g in &d.generics {
                    self.ident(g, Case::Type);
                }
                for f in &d.fields {
                    self.ident(&f.name, Case::Value);
                    self.ty(&f.ty);
                }
            }
            ItemKind::Enum(d) => {
                self.ident(&d.name, Case::Type);
                for g in &d.generics {
                    self.ident(g, Case::Type);
                }
                for v in &d.variants {
                    self.ident(&v.name, Case::Type);
                    for t in &v.payload {
                        self.ty(t);
                    }
                }
            }
            ItemKind::Fn(d) => self.fn_decl(d),
            ItemKind::Example(e) => {
                self.ident(&e.target, Case::Value);
                for a in &e.args {
                    self.expr(a);
                }
                self.expr(&e.expected);
            }
            ItemKind::Rules(r) => {
                self.ident(&r.name, Case::Type);
                for rel in &r.rels {
                    self.ident(&rel.name, Case::Value);
                    for c in &rel.cols {
                        self.ty(c);
                    }
                }
                for rule in &r.rules {
                    self.atom(&rule.head);
                    for b in &rule.body {
                        match b {
                            BodyAtom::Pred { atom, .. } => self.atom(atom),
                            BodyAtom::Cmp { lhs, rhs, .. } => {
                                self.term(lhs);
                                self.term(rhs);
                            }
                        }
                    }
                }
            }
            ItemKind::Machine(m) => {
                self.ident(&m.name, Case::Type);
                for c in &m.state_chains {
                    for link in &c.links {
                        for s in link {
                            self.ident(s, Case::Type);
                        }
                    }
                }
                for t in &m.transitions {
                    self.ident(&t.from, Case::Type);
                    self.ident(&t.to, Case::Type);
                    if let Some(g) = &t.guard {
                        self.expr(g);
                    }
                }
                for i in &m.invariants {
                    self.expr(&i.expr);
                }
            }
        }
    }

    fn fn_decl(&mut self, d: &FnDecl) {
        self.ident(&d.name, Case::Value);
        for g in &d.generics {
            self.ident(g, Case::Type);
        }
        for p in &d.params {
            self.ident(&p.name, Case::Value);
            self.ty(&p.ty);
        }
        self.ty(&d.ret);
        for c in &d.contracts {
            match c {
                Contract::Requires { expr, .. } | Contract::Decreases { expr, .. } => {
                    self.expr(expr)
                }
                Contract::Ensures { binder, expr, .. } => {
                    self.ident(binder, Case::Value);
                    self.expr(expr);
                }
            }
        }
        self.block(&d.body);
    }

    fn atom(&mut self, a: &RuleAtom) {
        self.ident(&a.name, Case::Value);
        for t in &a.terms {
            self.term(t);
        }
    }

    fn term(&mut self, t: &Term) {
        if let Term::Var(i) = t {
            self.ident(i, Case::Value);
        }
    }

    fn ty(&mut self, t: &Type) {
        if let TypeKind::Named { name, args } = &t.kind {
            self.ident(name, Case::Type);
            for a in args {
                self.ty(a);
            }
        }
    }

    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Let { name, ty, init, .. } => {
                self.ident(name, Case::Value);
                if let Some(t) = ty {
                    self.ty(t);
                }
                self.expr(init);
            }
            StmtKind::Assign { target, value } => {
                self.expr(target);
                self.expr(value);
            }
            StmtKind::While { cond, invariants, decreases, body } => {
                self.expr(cond);
                for i in invariants {
                    self.expr(i);
                }
                if let Some(d) = decreases {
                    self.expr(d);
                }
                self.block(body);
            }
            StmtKind::For { var, iter, body } => {
                self.ident(var, Case::Value);
                self.expr(iter);
                self.block(body);
            }
            StmtKind::Return(Some(e)) => self.expr(e),
            StmtKind::Return(None) => {}
            StmtKind::Expr(e) => self.expr(e),
        }
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Int(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::Unit
            | ExprKind::Dontcare
            | ExprKind::Unresolved { .. }
            | ExprKind::Error => {}
            // A bare name may be a value (snake_case) or a nullary variant (UpperCamel);
            // the resolver decides which. Only a spelling that is neither is a violation.
            ExprKind::Path(i) => {
                if !is_snake(&i.name) && !is_upper_camel(&i.name) {
                    self.ident(i, Case::Value);
                }
            }
            ExprKind::FieldRef(i) => self.ident(i, Case::Value),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Call { callee, type_args, args } => {
                self.expr(callee);
                for t in type_args {
                    self.ty(t);
                }
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::MethodCall { receiver, method, type_args, args } => {
                self.expr(receiver);
                self.ident(method, Case::Value);
                for t in type_args {
                    self.ty(t);
                }
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Field { base, name } => {
                self.expr(base);
                self.ident(name, Case::Value);
            }
            ExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
            }
            ExprKind::Block(b) => self.block(b),
            ExprKind::If { cond, then, else_ } => {
                self.expr(cond);
                self.block(then);
                if let Some(e) = else_ {
                    self.expr(e);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for a in arms {
                    self.pattern(&a.pattern);
                    self.expr(&a.body);
                }
            }
            ExprKind::List(items) => {
                for i in items {
                    self.expr(i);
                }
            }
            ExprKind::StructLit { name, fields } => {
                self.ident(name, Case::Type);
                for f in fields {
                    self.ident(&f.name, Case::Value);
                    self.expr(&f.value);
                }
            }
            ExprKind::Query(q) => {
                for f in &q.froms {
                    self.ident(&f.var, Case::Value);
                    self.expr(&f.source);
                }
                if let Some(w) = &q.filter {
                    self.expr(w);
                }
                if let Some(g) = &q.group {
                    self.ident(&g.row, Case::Value);
                    self.expr(&g.key);
                    self.ident(&g.binding, Case::Value);
                }
                self.expr(&q.select);
                if let Some(o) = &q.order {
                    self.expr(&o.key);
                }
                if let Some(h) = &q.hint {
                    self.ident(&h.key, Case::Value);
                    self.ident(&h.value, Case::Value);
                }
            }
        }
    }

    fn pattern(&mut self, p: &Pattern) {
        match &p.kind {
            PatternKind::Binding(i) => self.ident(i, Case::Value),
            PatternKind::Variant { name, args } => {
                self.ident(name, Case::Type);
                for a in args.iter().flatten() {
                    self.pattern(a);
                }
            }
            PatternKind::Struct { fields } => {
                for f in fields {
                    self.ident(&f.name, Case::Value);
                    if let Some(p) = &f.pattern {
                        self.pattern(p);
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------
// Case predicates and conversions
// ---------------------------------------------------------------------------------------

pub fn is_snake(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

pub fn is_upper_camel(s: &str) -> bool {
    !s.is_empty()
        && s.starts_with(|c: char| c.is_ascii_uppercase())
        && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Split an identifier into lowercase words, accepting either convention as input.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let starts_word = c.is_ascii_uppercase()
            && i > 0
            && (chars[i - 1].is_ascii_lowercase()
                || chars[i - 1].is_ascii_digit()
                || (i + 1 < chars.len() && chars[i + 1].is_ascii_lowercase()));
        if starts_word && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(c.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn to_snake(s: &str) -> String {
    let leading = s.len() - s.trim_start_matches('_').len();
    let body = words(s).join("_");
    format!("{}{}", "_".repeat(leading), body)
}

pub fn to_upper_camel(s: &str) -> String {
    words(s)
        .into_iter()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_conversions_round_trip() {
        assert_eq!(to_snake("totalFees"), "total_fees");
        assert_eq!(to_snake("HttpRequest"), "http_request");
        assert_eq!(to_snake("HTTPRequest"), "http_request");
        assert_eq!(to_snake("already_snake"), "already_snake");
        assert_eq!(to_snake("_private"), "_private");
        assert_eq!(to_upper_camel("http_request"), "HttpRequest");
        assert_eq!(to_upper_camel("AlreadyCamel"), "AlreadyCamel");
        assert_eq!(to_upper_camel("x"), "X");
    }

    #[test]
    fn predicates_agree_with_the_conversions() {
        for s in ["total_fees", "x", "a1", "_x"] {
            assert!(is_snake(s), "{s}");
            assert_eq!(to_snake(s), s);
        }
        for s in ["HttpRequest", "T", "List2"] {
            assert!(is_upper_camel(s), "{s}");
            assert_eq!(to_upper_camel(s), s);
        }
        assert!(!is_snake("totalFees"));
        assert!(!is_upper_camel("http_request"));
    }
}
