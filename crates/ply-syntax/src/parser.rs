//! Recursive-descent parser with Pratt expression parsing (§5.2).
//!
//! Error policy (§13, §16): one diagnostic per root cause. A hard parse error puts the
//! parser in panic mode, which suppresses further parse diagnostics until the next item
//! boundary; errors we can recover from precisely (an unknown capability, a borrow marker in
//! the wrong place) are reported without entering panic mode, so they never hide anything.

use crate::ast::*;
use crate::lexer::{Lexed, Token, TriviaKind, lex};
use crate::token::{TokenKind as T, quoted};
use ply_diag::{Code, Diagnostic, Diagnostics, Edit, FileId, Fix, Span};
use std::sync::Arc;

/// Parse a whole file.
pub fn parse_file(file: FileId, source: &Arc<str>, diags: &mut Diagnostics) -> File {
    let lexed = lex(file, source, diags);
    let mut p = Parser::new(&lexed, diags);
    p.parse_file()
}

/// Parse a single expression (used by tests, `ply explain` and the LSP).
pub fn parse_expression(file: FileId, source: &Arc<str>, diags: &mut Diagnostics) -> Expr {
    let lexed = lex(file, source, diags);
    let mut p = Parser::new(&lexed, diags);
    let e = p.parse_expr();
    if !p.at(T::Eof) {
        p.expected(T::Eof);
    }
    e
}

struct Parser<'a> {
    lexed: &'a Lexed,
    diags: &'a mut Diagnostics,
    pos: usize,
    /// Index of the first comment not yet attached to a node.
    trivia_cursor: usize,
    /// Suppresses cascading parse diagnostics until the next item boundary.
    panic: bool,
    /// Struct literals are not parsed in condition position (`if S { .. }`).
    no_struct_lit: bool,
}

impl<'a> Parser<'a> {
    fn new(lexed: &'a Lexed, diags: &'a mut Diagnostics) -> Parser<'a> {
        Parser { lexed, diags, pos: 0, trivia_cursor: 0, panic: false, no_struct_lit: false }
    }

    // -- token access -------------------------------------------------------------------

    fn tok(&self, n: usize) -> &'a Token {
        let i = (self.pos + n).min(self.lexed.tokens.len() - 1);
        &self.lexed.tokens[i]
    }

    fn cur(&self) -> &'a Token {
        self.tok(0)
    }

    fn peek(&self) -> T {
        self.cur().kind
    }

    fn peek_at(&self, n: usize) -> T {
        self.tok(n).kind
    }

    fn span(&self) -> Span {
        self.cur().span
    }

    fn prev_span(&self) -> Span {
        if self.pos == 0 { self.span() } else { self.lexed.tokens[self.pos - 1].span }
    }

    fn text(&self, n: usize) -> &'a str {
        self.lexed.text(self.tok(n))
    }

    fn at(&self, k: T) -> bool {
        self.peek() == k
    }

    /// A contextual keyword: an identifier with a specific spelling.
    fn at_kw(&self, kw: &str) -> bool {
        self.at(T::Ident) && self.text(0) == kw
    }

    fn bump(&mut self) -> &'a Token {
        let t = self.cur();
        if !self.at(T::Eof) {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, k: T) -> bool {
        if self.at(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    // -- diagnostics --------------------------------------------------------------------

    /// Report a hard error and enter panic mode.
    fn emit(&mut self, d: Diagnostic) {
        if self.panic {
            return;
        }
        self.panic = true;
        self.diags.push(d);
    }

    /// Report an error we can recover from exactly; does not suppress later diagnostics.
    fn emit_recoverable(&mut self, d: Diagnostic) {
        if self.panic {
            return;
        }
        self.diags.push(d);
    }

    fn expected(&mut self, kind: T) {
        let found = quoted(self.peek());
        let d = Diagnostic::new(
            Code::E0110,
            self.span(),
            format!("expected {}, found {}", quoted(kind), found),
        );
        self.emit(d);
    }

    fn expect(&mut self, kind: T) -> bool {
        if self.eat(kind) {
            true
        } else {
            self.expected(kind);
            false
        }
    }

    /// `;` gets its own message and an insertion fix — the single most common slip.
    fn expect_semi(&mut self) {
        if self.eat(T::Semi) {
            return;
        }
        let after = self.prev_span();
        let found = quoted(self.peek());
        let d = Diagnostic::new(Code::E0110, after.at_end(), format!("expected `;`, found {found}"))
            .with_fix(Fix::insert_after("add `;`", after, ";"));
        self.emit(d);
    }

    fn expect_close(&mut self, close: T, open: T, open_span: Span) -> bool {
        if self.eat(close) {
            return true;
        }
        let d = Diagnostic::new(
            Code::E0111,
            self.span(),
            format!("unclosed {} — expected {}, found {}", quoted(open), quoted(close), quoted(self.peek())),
        )
        .with_related(open_span, format!("{} opened here", quoted(open)))
        .with_fix(Fix::new(
            format!("close the `{}` here", open.as_str()),
            vec![Edit::insert(self.span(), close.as_str())],
        ));
        self.emit(d);
        false
    }

    // -- comment trivia -----------------------------------------------------------------

    fn make_comment(&self, idx: usize) -> Comment {
        let t = &self.lexed.trivia[idx];
        Comment {
            text: self.lexed.trivia_text(t).trim_end().to_string(),
            span: t.span,
            block: t.kind == TriviaKind::Block,
            blank_before: t.newlines_before >= 2,
        }
    }

    /// Claim every comment written before the current token as this node's leading comments.
    fn take_comments(&mut self) -> Comments {
        let end = self.cur().leading().end;
        let mut leading = Vec::new();
        while self.trivia_cursor < end {
            leading.push(self.make_comment(self.trivia_cursor));
            self.trivia_cursor += 1;
        }
        let blank_before = match leading.first() {
            Some(c) => c.blank_before,
            None => self.cur().newlines_before >= 2,
        };
        Comments { leading, trailing: None, blank_before }
    }

    /// Claim a comment written after a node on the same line.
    fn take_trailing(&mut self, into: &mut Comments) {
        let range = self.cur().leading();
        if self.trivia_cursor != range.start || self.trivia_cursor >= range.end {
            return;
        }
        if self.lexed.trivia[self.trivia_cursor].newlines_before != 0 {
            return;
        }
        into.trailing = Some(self.make_comment(self.trivia_cursor));
        self.trivia_cursor += 1;
    }

    // -- recovery -----------------------------------------------------------------------

    fn sync_to_item(&mut self) {
        loop {
            match self.peek() {
                T::Eof => return,
                k if k.starts_item() => return,
                T::Ident if self.text(0) == "example" => return,
                T::RBrace => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn sync_to_stmt(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                T::Eof => return,
                T::Semi if depth == 0 => {
                    self.bump();
                    return;
                }
                T::RBrace if depth == 0 => return,
                T::LBrace | T::LParen | T::LBracket => {
                    depth += 1;
                    self.bump();
                }
                T::RBrace | T::RParen | T::RBracket => {
                    depth -= 1;
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    // -- items --------------------------------------------------------------------------

    fn parse_file(&mut self) -> File {
        let mut items = Vec::new();
        loop {
            self.panic = false;
            let comments = self.take_comments();
            if self.at(T::Eof) {
                return File { items, trailing_comments: comments.leading };
            }
            let before = self.pos;
            match self.parse_item(comments) {
                Some(item) => items.push(item),
                None => {
                    if self.pos == before {
                        self.bump();
                    }
                    self.sync_to_item();
                }
            }
        }
    }

    fn parse_item(&mut self, comments: Comments) -> Option<Item> {
        let start = self.span();
        let kind = match self.peek() {
            T::Use => {
                self.bump();
                let name = self.parse_ident()?;
                self.expect_semi();
                ItemKind::Use(UseDecl { name })
            }
            T::Struct => ItemKind::Struct(self.parse_struct()?),
            T::Enum => ItemKind::Enum(self.parse_enum()?),
            T::Fn => ItemKind::Fn(Box::new(self.parse_fn()?)),
            T::Rules => ItemKind::Rules(self.parse_rules()?),
            T::Machine => ItemKind::Machine(self.parse_machine()?),
            T::Ident if self.text(0) == "example" => ItemKind::Example(self.parse_example()?),
            _ => {
                let found = quoted(self.peek());
                let d = Diagnostic::new(
                    Code::E0116,
                    self.span(),
                    format!("expected an item (`fn`, `struct`, `enum`, `use`, `rules`, `machine`), found {found}"),
                );
                self.emit(d);
                return None;
            }
        };
        let mut comments = comments;
        self.take_trailing(&mut comments);
        Some(Item { kind, span: start.to(self.prev_span()), comments })
    }

    fn parse_ident(&mut self) -> Option<Ident> {
        if self.at(T::Ident) {
            let t = self.bump();
            Some(Ident::new(self.lexed.text(t), t.span))
        } else {
            self.expected(T::Ident);
            None
        }
    }

    fn parse_generics(&mut self) -> Vec<Ident> {
        let mut out = Vec::new();
        if !self.at(T::LBracket) {
            return out;
        }
        let open = self.span();
        self.bump();
        loop {
            if self.at(T::RBracket) || self.at(T::Eof) {
                break;
            }
            match self.parse_ident() {
                Some(i) => out.push(i),
                None => break,
            }
            if !self.eat(T::Comma) {
                break;
            }
        }
        self.expect_close(T::RBracket, T::LBracket, open);
        out
    }

    fn parse_struct(&mut self) -> Option<StructDecl> {
        self.bump(); // struct
        let name = self.parse_ident()?;
        let generics = self.parse_generics();
        let open = self.span();
        if !self.expect(T::LBrace) {
            return None;
        }
        let mut fields = Vec::new();
        loop {
            let comments = self.take_comments();
            if self.at(T::RBrace) || self.at(T::Eof) {
                break;
            }
            let start = self.span();
            let Some(fname) = self.parse_ident() else { break };
            self.expect(T::Colon);
            let ty = self.parse_type();
            let mut comments = comments;
            let span = start.to(self.prev_span());
            let sep = self.eat(T::Comma);
            self.take_trailing(&mut comments);
            fields.push(Field { name: fname, ty, span, comments });
            if !sep {
                break;
            }
        }
        self.expect_close(T::RBrace, T::LBrace, open);
        Some(StructDecl { name, generics, fields })
    }

    fn parse_enum(&mut self) -> Option<EnumDecl> {
        self.bump(); // enum
        let name = self.parse_ident()?;
        let generics = self.parse_generics();
        let open = self.span();
        if !self.expect(T::LBrace) {
            return None;
        }
        let mut variants = Vec::new();
        loop {
            let comments = self.take_comments();
            if self.at(T::RBrace) || self.at(T::Eof) {
                break;
            }
            let start = self.span();
            let Some(vname) = self.parse_ident() else { break };
            let mut payload = Vec::new();
            if self.at(T::LParen) {
                let popen = self.span();
                self.bump();
                loop {
                    if self.at(T::RParen) || self.at(T::Eof) {
                        break;
                    }
                    payload.push(self.parse_type());
                    if !self.eat(T::Comma) {
                        break;
                    }
                }
                self.expect_close(T::RParen, T::LParen, popen);
            }
            let mut comments = comments;
            let span = start.to(self.prev_span());
            let sep = self.eat(T::Comma);
            self.take_trailing(&mut comments);
            variants.push(Variant { name: vname, payload, span, comments });
            if !sep {
                break;
            }
        }
        self.expect_close(T::RBrace, T::LBrace, open);
        Some(EnumDecl { name, generics, variants })
    }

    fn parse_fn(&mut self) -> Option<FnDecl> {
        self.bump(); // fn
        let name = self.parse_ident()?;
        let generics = self.parse_generics();
        let popen = self.span();
        if !self.expect(T::LParen) {
            return None;
        }
        let mut params = Vec::new();
        loop {
            let comments = self.take_comments();
            if self.at(T::RParen) || self.at(T::Eof) {
                break;
            }
            let start = self.span();
            let Some(pname) = self.parse_ident() else { break };
            let (mode, ty) = if self.eat(T::Colon) {
                self.parse_ptype()
            } else {
                let d = Diagnostic::new(
                    Code::E0113,
                    start,
                    format!("parameter `{}` needs a type annotation, e.g. `{}: Int`", pname.name, pname.name),
                );
                self.emit_recoverable(d);
                (ParamMode::Owned, Type { kind: TypeKind::Error, span: start })
            };
            let mut comments = comments;
            let span = start.to(self.prev_span());
            let sep = self.eat(T::Comma);
            self.take_trailing(&mut comments);
            params.push(Param { name: pname, mode, ty, span, comments });
            if !sep {
                break;
            }
        }
        let close_paren = self.span();
        self.expect_close(T::RParen, T::LParen, popen);

        let ret = if self.eat(T::Arrow) {
            self.parse_type()
        } else {
            let after = self.prev_span();
            let d = Diagnostic::new(
                Code::E0112,
                after.at_end(),
                "every function needs a return type; write `-> ()` for a function that returns nothing",
            )
            .with_fix(Fix::insert_after("return `()`", after, " -> ()"));
            self.emit_recoverable(d);
            let _ = close_paren;
            Type { kind: TypeKind::Unit, span: after.at_end() }
        };

        let uses = self.parse_uses();
        let contracts = self.parse_contracts();
        let verify = self.parse_verify();
        let body = self.parse_block();
        Some(FnDecl { name, generics, params, ret, uses, contracts, verify, body })
    }

    fn parse_uses(&mut self) -> Option<UsesClause> {
        if !self.at(T::Uses) {
            return None;
        }
        let start = self.span();
        self.bump();
        let open = self.span();
        self.expect(T::LBrace);
        let mut caps = Vec::new();
        loop {
            if self.at(T::RBrace) || self.at(T::Eof) {
                break;
            }
            let cstart = self.span();
            let mut text = String::new();
            loop {
                if self.at(T::Ident) {
                    text.push_str(self.text(0));
                    self.bump();
                } else {
                    self.expected(T::Ident);
                    break;
                }
                if self.eat(T::Dot) {
                    text.push('.');
                } else {
                    break;
                }
            }
            let span = cstart.to(self.prev_span());
            match Cap::from_str(&text) {
                Some(cap) => caps.push(Capability { cap, span }),
                None => {
                    let names: Vec<&str> = Cap::ALL.iter().map(|c| c.as_str()).collect();
                    let d = Diagnostic::new(
                        Code::E0119,
                        span,
                        format!("unknown capability `{text}`; Ply v0 has {}", names.join(", ")),
                    );
                    self.emit_recoverable(d);
                }
            }
            if !self.eat(T::Comma) {
                break;
            }
        }
        self.expect_close(T::RBrace, T::LBrace, open);
        Some(UsesClause { caps, span: start.to(self.prev_span()) })
    }

    fn parse_contracts(&mut self) -> Vec<Contract> {
        let mut out = Vec::new();
        loop {
            let comments = self.take_comments();
            let start = self.span();
            match self.peek() {
                T::Requires => {
                    self.bump();
                    let expr = self.parse_expr();
                    let mut comments = comments;
                    let span = start.to(self.prev_span());
                    self.take_trailing(&mut comments);
                    out.push(Contract::Requires { expr, span, comments });
                }
                T::Ensures => {
                    self.bump();
                    self.expect(T::Pipe);
                    let binder = self
                        .parse_ident()
                        .unwrap_or_else(|| Ident::new("r", self.prev_span()));
                    self.expect(T::Pipe);
                    let expr = self.parse_expr();
                    let mut comments = comments;
                    let span = start.to(self.prev_span());
                    self.take_trailing(&mut comments);
                    out.push(Contract::Ensures { binder, expr, span, comments });
                }
                T::Decreases => {
                    self.bump();
                    let expr = self.parse_expr();
                    let mut comments = comments;
                    let span = start.to(self.prev_span());
                    self.take_trailing(&mut comments);
                    out.push(Contract::Decreases { expr, span, comments });
                }
                _ => {
                    // Nothing consumed: hand the comments back to whoever comes next.
                    self.trivia_cursor -= comments.leading.len();
                    return out;
                }
            }
        }
    }

    fn parse_verify(&mut self) -> Option<VerifyClause> {
        if !self.at(T::Verify) {
            return None;
        }
        let start = self.span();
        self.bump();
        let word_span = self.span();
        let word = if self.at(T::Ident) { self.text(0).to_string() } else { String::new() };
        if word.is_empty() {
            self.expected(T::Ident);
            return Some(VerifyClause { mode: VerifyMode::Test, span: start.to(self.prev_span()) });
        }
        self.bump();
        let mode = match word.as_str() {
            "test" => VerifyMode::Test,
            "prove" => VerifyMode::Prove,
            "fuzz" => VerifyMode::Fuzz { runs: self.parse_verify_arg("runs") },
            "bounded" => VerifyMode::Bounded { depth: self.parse_verify_arg("depth") },
            "induct" => VerifyMode::Induct { k: self.parse_verify_arg("k") },
            other => {
                let d = Diagnostic::new(
                    Code::E0118,
                    word_span,
                    format!(
                        "unknown verification mode `{other}`; the dial is test, fuzz, bounded, induct, prove"
                    ),
                );
                self.emit_recoverable(d);
                VerifyMode::Test
            }
        };
        Some(VerifyClause { mode, span: start.to(self.prev_span()) })
    }

    /// `(runs: 256)` — the whole parenthesised part is optional.
    fn parse_verify_arg(&mut self, key: &str) -> Option<u64> {
        if !self.at(T::LParen) {
            return None;
        }
        let open = self.span();
        self.bump();
        if self.at(T::Ident) && self.text(0) == key {
            self.bump();
        } else {
            let d = Diagnostic::new(
                Code::E0118,
                self.span(),
                format!("expected `{key}:` here"),
            );
            self.emit_recoverable(d);
        }
        self.expect(T::Colon);
        let value = if self.at(T::Int) {
            let t = self.bump();
            self.lexed.text(t).replace('_', "").parse::<u64>().ok()
        } else {
            self.expected(T::Int);
            None
        };
        self.expect_close(T::RParen, T::LParen, open);
        value
    }

    fn parse_example(&mut self) -> Option<ExampleDecl> {
        self.bump(); // example
        let target = self.parse_ident()?;
        let open = self.span();
        self.expect(T::LParen);
        let mut args = Vec::new();
        loop {
            if self.at(T::RParen) || self.at(T::Eof) {
                break;
            }
            args.push(self.parse_expr());
            if !self.eat(T::Comma) {
                break;
            }
        }
        self.expect_close(T::RParen, T::LParen, open);
        self.expect(T::EqEq);
        let expected = self.parse_expr();
        self.expect_semi();
        Some(ExampleDecl { target, args, expected })
    }

    // -- types --------------------------------------------------------------------------

    /// A parameter type, which may carry `&` / `&mut` (§5.5).
    fn parse_ptype(&mut self) -> (ParamMode, Type) {
        if self.at(T::Amp) {
            self.bump();
            let mode = if self.eat(T::Mut) { ParamMode::RefMut } else { ParamMode::Ref };
            (mode, self.parse_type())
        } else {
            (ParamMode::Owned, self.parse_type())
        }
    }

    fn parse_type(&mut self) -> Type {
        if self.at(T::Amp) {
            let amp = self.span();
            self.bump();
            let end = if self.at(T::Mut) { self.span() } else { amp };
            self.eat(T::Mut);
            let d = Diagnostic::new(
                Code::E0123,
                amp.to(end),
                "borrows exist only as parameters: they cannot appear in fields, return types or `let` bindings (§5.5)",
            )
            .with_fix(Fix::new(
                "drop the borrow marker",
                vec![Edit::delete(amp.to(self.span().at_start()))],
            ));
            self.emit_recoverable(d);
            return self.parse_type();
        }
        let start = self.span();
        if self.at(T::LParen) {
            self.bump();
            self.expect(T::RParen);
            return Type { kind: TypeKind::Unit, span: start.to(self.prev_span()) };
        }
        let Some(name) = self.parse_ident() else {
            return Type { kind: TypeKind::Error, span: start };
        };
        let mut args = Vec::new();
        if self.at(T::LBracket) {
            let open = self.span();
            self.bump();
            loop {
                if self.at(T::RBracket) || self.at(T::Eof) {
                    break;
                }
                args.push(self.parse_type());
                if !self.eat(T::Comma) {
                    break;
                }
            }
            self.expect_close(T::RBracket, T::LBracket, open);
        }
        Type { kind: TypeKind::Named { name, args }, span: start.to(self.prev_span()) }
    }

    // -- blocks and statements ----------------------------------------------------------

    fn parse_block(&mut self) -> Block {
        let open = self.span();
        if !self.expect(T::LBrace) {
            return Block { stmts: Vec::new(), tail: None, trailing_comments: Vec::new(), span: open };
        }
        let outer_no_struct = std::mem::replace(&mut self.no_struct_lit, false);
        let mut stmts = Vec::new();
        let mut tail = None;
        let mut trailing_comments = Vec::new();
        loop {
            let comments = self.take_comments();
            if self.at(T::RBrace) || self.at(T::Eof) {
                trailing_comments = comments.leading;
                break;
            }
            let before = self.pos;
            match self.parse_stmt(comments) {
                StmtOrTail::Stmt(s) => stmts.push(s),
                StmtOrTail::Tail(e) => {
                    tail = Some(Box::new(e));
                    break;
                }
                StmtOrTail::Failed => {
                    if self.pos == before {
                        self.bump();
                    }
                    self.sync_to_stmt();
                }
            }
        }
        self.expect_close(T::RBrace, T::LBrace, open);
        self.no_struct_lit = outer_no_struct;
        Block { stmts, tail, trailing_comments, span: open.to(self.prev_span()) }
    }

    fn parse_stmt(&mut self, comments: Comments) -> StmtOrTail {
        let start = self.span();
        let kind = match self.peek() {
            T::Let => {
                self.bump();
                let mutable = self.eat(T::Mut);
                let Some(name) = self.parse_ident() else { return StmtOrTail::Failed };
                let ty = if self.eat(T::Colon) { Some(self.parse_type()) } else { None };
                self.expect(T::Eq);
                let init = self.parse_expr();
                self.expect_semi();
                StmtKind::Let { mutable, name, ty, init }
            }
            T::While => {
                self.bump();
                let cond = self.parse_cond_expr();
                let mut invariants = Vec::new();
                while self.at(T::Invariant) {
                    self.bump();
                    invariants.push(self.parse_cond_expr());
                }
                let decreases = if self.eat(T::Decreases) {
                    Some(self.parse_cond_expr())
                } else {
                    None
                };
                let body = self.parse_block();
                StmtKind::While { cond, invariants, decreases, body }
            }
            T::For => {
                self.bump();
                let Some(var) = self.parse_ident() else { return StmtOrTail::Failed };
                self.expect(T::In);
                let iter = self.parse_cond_expr();
                let body = self.parse_block();
                StmtKind::For { var, iter, body }
            }
            T::Return => {
                self.bump();
                let value = if self.at(T::Semi) { None } else { Some(self.parse_expr()) };
                self.expect_semi();
                StmtKind::Return(value)
            }
            _ => {
                let expr = self.parse_expr();
                if matches!(expr.kind, ExprKind::Error) {
                    return StmtOrTail::Failed;
                }
                if self.at(T::Eq) {
                    self.bump();
                    let value = self.parse_expr();
                    self.expect_semi();
                    if !is_place(&expr) {
                        let d = Diagnostic::new(
                            Code::E0114,
                            expr.span,
                            "only a local, a field or an index can be assigned to",
                        );
                        self.emit_recoverable(d);
                    }
                    StmtKind::Assign { target: expr, value }
                } else if self.eat(T::Semi) {
                    StmtKind::Expr(expr)
                } else if self.at(T::RBrace) || self.at(T::Eof) {
                    let mut comments = comments;
                    self.take_trailing(&mut comments);
                    return StmtOrTail::Tail(expr);
                } else if is_block_like(&expr) {
                    StmtKind::Expr(expr)
                } else {
                    self.expect_semi();
                    StmtKind::Expr(expr)
                }
            }
        };
        let mut comments = comments;
        let span = start.to(self.prev_span());
        self.take_trailing(&mut comments);
        StmtOrTail::Stmt(Stmt { kind, span, comments })
    }

    // -- expressions --------------------------------------------------------------------

    fn parse_expr(&mut self) -> Expr {
        self.parse_expr_bp(0)
    }

    /// Condition position: struct literals are off, because `{` starts the body.
    fn parse_cond_expr(&mut self) -> Expr {
        let saved = std::mem::replace(&mut self.no_struct_lit, true);
        let e = self.parse_expr_bp(0);
        self.no_struct_lit = saved;
        e
    }

    /// Argument position: struct literals are back on.
    fn parse_nested_expr(&mut self) -> Expr {
        let saved = std::mem::replace(&mut self.no_struct_lit, false);
        let e = self.parse_expr_bp(0);
        self.no_struct_lit = saved;
        e
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Expr {
        let mut lhs = self.parse_unary();
        loop {
            if let Some((op, word, doubled)) = match (self.peek(), self.peek_at(1)) {
                (T::Amp, T::Amp) => Some((BinOp::And, "and", "&&")),
                (T::Pipe, T::Pipe) => Some((BinOp::Or, "or", "||")),
                _ => None,
            } {
                let bp = op.precedence();
                if bp < min_bp {
                    break;
                }
                let span = self.span().to(self.tok(1).span);
                let d = Diagnostic::new(
                    Code::E0110,
                    span,
                    format!(
                        "`{doubled}` is not an operator in Ply; the boolean operators are `and`, `or`, `not`"
                    ),
                )
                .with_fix(Fix::replace(format!("use `{word}`"), span, word));
                self.emit_recoverable(d);
                self.bump();
                self.bump();
                let rhs = self.parse_expr_bp(bp + 1);
                lhs = Expr {
                    span: lhs.span.to(rhs.span),
                    kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) },
                };
                continue;
            }
            if self.no_struct_lit && self.at(T::Eq) {
                // `if x = 1 { .. }` — a mistyped comparison; recover as `==`.
                let span = self.span();
                let d = Diagnostic::new(Code::E0110, span, "`=` assigns; use `==` to compare")
                    .with_fix(Fix::replace("use `==`", span, "=="));
                self.emit_recoverable(d);
                self.bump();
                let rhs = self.parse_expr_bp(BinOp::Eq.precedence() + 1);
                lhs = Expr {
                    span: lhs.span.to(rhs.span),
                    kind: ExprKind::Binary { op: BinOp::Eq, lhs: Box::new(lhs), rhs: Box::new(rhs) },
                };
                continue;
            }
            let Some(op) = binop_of(self.peek()) else { break };
            let bp = op.precedence();
            if bp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_expr_bp(bp + 1);
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) },
            };
            if op.is_comparison()
                && binop_of(self.peek()).is_some_and(|next| next.precedence() == bp)
            {
                let d = Diagnostic::new(
                    Code::E0110,
                    self.span(),
                    format!(
                        "`{}` does not chain; write `a {} b and b {} c`",
                        op.as_str(),
                        op.as_str(),
                        op.as_str()
                    ),
                );
                self.emit(d);
                break;
            }
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        let start = self.span();
        let op = match self.peek() {
            T::Not | T::Bang => Some(UnOp::Not),
            T::Minus => Some(UnOp::Neg),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let operand = self.parse_unary();
            return Expr {
                span: start.to(operand.span),
                kind: ExprKind::Unary { op, operand: Box::new(operand) },
            };
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut e = self.parse_primary();
        loop {
            match self.peek() {
                T::LParen => {
                    let args = self.parse_call_args();
                    let span = e.span.to(self.prev_span());
                    e = Expr {
                        span,
                        kind: ExprKind::Call { callee: Box::new(e), type_args: Vec::new(), args },
                    };
                }
                T::LBracket if self.looks_like_type_args() => {
                    let type_args = self.parse_type_args();
                    let args = self.parse_call_args();
                    let span = e.span.to(self.prev_span());
                    e = match e.kind {
                        ExprKind::MethodCall { receiver, method, args: old, .. } if old.is_empty() => {
                            Expr {
                                span,
                                kind: ExprKind::MethodCall { receiver, method, type_args, args },
                            }
                        }
                        _ => Expr {
                            span,
                            kind: ExprKind::Call { callee: Box::new(e), type_args, args },
                        },
                    };
                }
                T::LBracket => {
                    let open = self.span();
                    self.bump();
                    let index = self.parse_nested_expr();
                    self.expect_close(T::RBracket, T::LBracket, open);
                    let span = e.span.to(self.prev_span());
                    e = Expr {
                        span,
                        kind: ExprKind::Index { base: Box::new(e), index: Box::new(index) },
                    };
                }
                T::Dot => {
                    self.bump();
                    let Some(name) = self.parse_ident() else { return e };
                    if self.at(T::LParen) {
                        let args = self.parse_call_args();
                        let span = e.span.to(self.prev_span());
                        e = Expr {
                            span,
                            kind: ExprKind::MethodCall {
                                receiver: Box::new(e),
                                method: name,
                                type_args: Vec::new(),
                                args,
                            },
                        };
                    } else if self.at(T::LBracket) && self.looks_like_type_args() {
                        let type_args = self.parse_type_args();
                        let args = self.parse_call_args();
                        let span = e.span.to(self.prev_span());
                        e = Expr {
                            span,
                            kind: ExprKind::MethodCall {
                                receiver: Box::new(e),
                                method: name,
                                type_args,
                                args,
                            },
                        };
                    } else {
                        let span = e.span.to(name.span);
                        e = Expr { span, kind: ExprKind::Field { base: Box::new(e), name } };
                    }
                }
                _ => return e,
            }
        }
    }

    fn parse_call_args(&mut self) -> Vec<Expr> {
        let open = self.span();
        if !self.expect(T::LParen) {
            return Vec::new();
        }
        let mut args = Vec::new();
        loop {
            if self.at(T::RParen) || self.at(T::Eof) {
                break;
            }
            args.push(self.parse_nested_expr());
            if !self.eat(T::Comma) {
                break;
            }
        }
        self.expect_close(T::RParen, T::LParen, open);
        args
    }

    fn parse_type_args(&mut self) -> Vec<Type> {
        let open = self.span();
        self.bump(); // [
        let mut out = Vec::new();
        loop {
            if self.at(T::RBracket) || self.at(T::Eof) {
                break;
            }
            out.push(self.parse_type());
            if !self.eat(T::Comma) {
                break;
            }
        }
        self.expect_close(T::RBracket, T::LBracket, open);
        out
    }

    /// `f[Int](x)` is a call with explicit type arguments; `xs[i]` is an index. The
    /// bracketed group must contain only type syntax and be followed by `(`.
    fn looks_like_type_args(&self) -> bool {
        debug_assert!(self.at(T::LBracket));
        let mut depth = 0usize;
        let mut i = 0usize;
        let mut saw_type_name = false;
        loop {
            let k = self.peek_at(i);
            match k {
                T::LBracket => depth += 1,
                T::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                T::Ident => {
                    let text = self.text(i);
                    if !text.starts_with(|c: char| c.is_ascii_uppercase()) {
                        return false;
                    }
                    saw_type_name = true;
                }
                T::Comma | T::LParen | T::RParen => {}
                _ => return false,
            }
            i += 1;
            if i > 64 {
                return false;
            }
        }
        saw_type_name && self.peek_at(i + 1) == T::LParen
    }

    fn parse_primary(&mut self) -> Expr {
        let start = self.span();
        match self.peek() {
            T::Int => {
                let t = self.bump();
                Expr { span: t.span, kind: ExprKind::Int(self.lexed.text(t).replace('_', "")) }
            }
            T::Str => {
                let t = self.bump();
                Expr { span: t.span, kind: ExprKind::Str(self.lexed.string_value(t).to_string()) }
            }
            T::True => {
                self.bump();
                Expr { span: start, kind: ExprKind::Bool(true) }
            }
            T::False => {
                self.bump();
                Expr { span: start, kind: ExprKind::Bool(false) }
            }
            T::Dontcare => {
                self.bump();
                Expr { span: start, kind: ExprKind::Dontcare }
            }
            T::Unresolved => {
                self.bump();
                self.expect(T::Hash);
                let id = if self.at(T::Int) {
                    let t = self.bump();
                    self.lexed.text(t).replace('_', "").parse::<u64>().unwrap_or(0)
                } else {
                    self.expected(T::Int);
                    0
                };
                Expr { span: start.to(self.prev_span()), kind: ExprKind::Unresolved { id } }
            }
            T::LParen => {
                self.bump();
                if self.eat(T::RParen) {
                    return Expr { span: start.to(self.prev_span()), kind: ExprKind::Unit };
                }
                let inner = self.parse_nested_expr();
                self.expect_close(T::RParen, T::LParen, start);
                inner
            }
            T::LBracket => {
                self.bump();
                let mut items = Vec::new();
                loop {
                    if self.at(T::RBracket) || self.at(T::Eof) {
                        break;
                    }
                    items.push(self.parse_nested_expr());
                    if !self.eat(T::Comma) {
                        break;
                    }
                }
                self.expect_close(T::RBracket, T::LBracket, start);
                Expr { span: start.to(self.prev_span()), kind: ExprKind::List(items) }
            }
            T::LBrace => {
                let b = self.parse_block();
                Expr { span: b.span, kind: ExprKind::Block(Box::new(b)) }
            }
            T::If => self.parse_if(),
            T::Match => self.parse_match(),
            T::Query => self.parse_query(),
            T::Dot if self.peek_at(1) == T::Ident => {
                self.bump();
                let name = self.parse_ident().expect("checked");
                Expr { span: start.to(name.span), kind: ExprKind::FieldRef(name) }
            }
            T::Ident => {
                let t = self.bump();
                let ident = Ident::new(self.lexed.text(t), t.span);
                if self.at(T::LBrace) && !self.no_struct_lit && ident.looks_like_type() {
                    return self.parse_struct_lit(ident);
                }
                Expr { span: ident.span, kind: ExprKind::Path(ident) }
            }
            _ => {
                let found = quoted(self.peek());
                let d = Diagnostic::new(
                    Code::E0115,
                    self.span(),
                    format!("expected an expression, found {found}"),
                );
                self.emit(d);
                Expr { span: start, kind: ExprKind::Error }
            }
        }
    }

    fn parse_struct_lit(&mut self, name: Ident) -> Expr {
        let open = self.span();
        self.bump(); // {
        let saved = std::mem::replace(&mut self.no_struct_lit, false);
        let mut fields = Vec::new();
        loop {
            let comments = self.take_comments();
            if self.at(T::RBrace) || self.at(T::Eof) {
                break;
            }
            let fstart = self.span();
            let Some(fname) = self.parse_ident() else { break };
            self.expect(T::Colon);
            let value = self.parse_expr();
            let mut comments = comments;
            let span = fstart.to(self.prev_span());
            let sep = self.eat(T::Comma);
            self.take_trailing(&mut comments);
            fields.push(FieldInit { name: fname, value, span, comments });
            if !sep {
                break;
            }
        }
        self.expect_close(T::RBrace, T::LBrace, open);
        self.no_struct_lit = saved;
        Expr { span: name.span.to(self.prev_span()), kind: ExprKind::StructLit { name, fields } }
    }

    fn parse_if(&mut self) -> Expr {
        let start = self.span();
        self.bump(); // if
        let cond = self.parse_cond_expr();
        let then = self.parse_block();
        let else_ = if self.eat(T::Else) {
            if self.at(T::If) {
                Some(Box::new(self.parse_if()))
            } else {
                let b = self.parse_block();
                Some(Box::new(Expr { span: b.span, kind: ExprKind::Block(Box::new(b)) }))
            }
        } else {
            None
        };
        Expr {
            span: start.to(self.prev_span()),
            kind: ExprKind::If { cond: Box::new(cond), then: Box::new(then), else_ },
        }
    }

    fn parse_match(&mut self) -> Expr {
        let start = self.span();
        self.bump(); // match
        let scrutinee = self.parse_cond_expr();
        let open = self.span();
        self.expect(T::LBrace);
        let saved = std::mem::replace(&mut self.no_struct_lit, false);
        let mut arms = Vec::new();
        loop {
            let comments = self.take_comments();
            if self.at(T::RBrace) || self.at(T::Eof) {
                break;
            }
            let astart = self.span();
            let pattern = self.parse_pattern();
            self.expect(T::FatArrow);
            let body = self.parse_expr();
            let mut comments = comments;
            let span = astart.to(self.prev_span());
            self.eat(T::Comma);
            self.take_trailing(&mut comments);
            arms.push(Arm { pattern, body, span, comments });
        }
        self.expect_close(T::RBrace, T::LBrace, open);
        self.no_struct_lit = saved;
        Expr {
            span: start.to(self.prev_span()),
            kind: ExprKind::Match { scrutinee: Box::new(scrutinee), arms },
        }
    }

    // -- patterns -----------------------------------------------------------------------

    fn parse_pattern(&mut self) -> Pattern {
        let start = self.span();
        match self.peek() {
            T::Int => {
                let t = self.bump();
                Pattern { span: t.span, kind: PatternKind::Int(self.lexed.text(t).replace('_', "")) }
            }
            T::Minus if self.peek_at(1) == T::Int => {
                self.bump();
                let t = self.bump();
                Pattern {
                    span: start.to(t.span),
                    kind: PatternKind::Int(format!("-{}", self.lexed.text(t).replace('_', ""))),
                }
            }
            T::Str => {
                let t = self.bump();
                Pattern { span: t.span, kind: PatternKind::Str(self.lexed.string_value(t).to_string()) }
            }
            T::True => {
                self.bump();
                Pattern { span: start, kind: PatternKind::Bool(true) }
            }
            T::False => {
                self.bump();
                Pattern { span: start, kind: PatternKind::Bool(false) }
            }
            T::LParen => {
                self.bump();
                self.expect(T::RParen);
                Pattern { span: start.to(self.prev_span()), kind: PatternKind::Unit }
            }
            T::LBrace => {
                self.bump();
                let mut fields = Vec::new();
                loop {
                    if self.at(T::RBrace) || self.at(T::Eof) {
                        break;
                    }
                    let fstart = self.span();
                    let Some(name) = self.parse_ident() else { break };
                    let pattern = if self.eat(T::Colon) { Some(self.parse_pattern()) } else { None };
                    fields.push(FieldPattern { name, pattern, span: fstart.to(self.prev_span()) });
                    if !self.eat(T::Comma) {
                        break;
                    }
                }
                self.expect_close(T::RBrace, T::LBrace, start);
                Pattern { span: start.to(self.prev_span()), kind: PatternKind::Struct { fields } }
            }
            T::Ident => {
                let t = self.bump();
                let name = Ident::new(self.lexed.text(t), t.span);
                if name.name == "_" {
                    return Pattern { span: name.span, kind: PatternKind::Wildcard };
                }
                if !name.looks_like_type() {
                    return Pattern { span: name.span, kind: PatternKind::Binding(name) };
                }
                let args = if self.at(T::LParen) {
                    let open = self.span();
                    self.bump();
                    let mut sub = Vec::new();
                    loop {
                        if self.at(T::RParen) || self.at(T::Eof) {
                            break;
                        }
                        sub.push(self.parse_pattern());
                        if !self.eat(T::Comma) {
                            break;
                        }
                    }
                    self.expect_close(T::RParen, T::LParen, open);
                    Some(sub)
                } else {
                    None
                };
                Pattern {
                    span: start.to(self.prev_span()),
                    kind: PatternKind::Variant { name, args },
                }
            }
            _ => {
                let found = quoted(self.peek());
                let d = Diagnostic::new(
                    Code::E0117,
                    self.span(),
                    format!("expected a pattern, found {found}"),
                );
                self.emit(d);
                Pattern { span: start, kind: PatternKind::Error }
            }
        }
    }

    // -- query --------------------------------------------------------------------------

    fn parse_query(&mut self) -> Expr {
        let start = self.span();
        self.bump(); // query
        let open = self.span();
        self.expect(T::LBrace);
        let saved = std::mem::replace(&mut self.no_struct_lit, false);

        let mut froms = Vec::new();
        loop {
            let comments = self.take_comments();
            if !self.at(T::From) {
                if froms.is_empty() {
                    let d = Diagnostic::new(
                        Code::E0120,
                        self.span(),
                        "a query starts with `from <var> in <list>`",
                    );
                    self.emit_recoverable(d);
                }
                self.trivia_cursor -= comments.leading.len();
                break;
            }
            let fstart = self.span();
            self.bump();
            let var = self.parse_ident().unwrap_or_else(|| Ident::new("_", self.prev_span()));
            self.expect(T::In);
            let source = self.parse_expr();
            let mut comments = comments;
            let span = fstart.to(self.prev_span());
            self.take_trailing(&mut comments);
            froms.push(FromClause { var, source, span, comments });
            if !self.eat(T::Comma) {
                break;
            }
        }

        let filter = if self.eat(T::Where) { Some(self.parse_expr()) } else { None };

        let group = if self.at(T::Group) {
            let gstart = self.span();
            self.bump();
            let row = self.parse_ident().unwrap_or_else(|| Ident::new("_", self.prev_span()));
            self.expect(T::By);
            let key = self.parse_expr();
            self.expect(T::Into);
            let binding = self.parse_ident().unwrap_or_else(|| Ident::new("g", self.prev_span()));
            Some(GroupClause { row, key, binding, span: gstart.to(self.prev_span()) })
        } else {
            None
        };

        let select = if self.eat(T::Select) {
            self.parse_expr()
        } else {
            let d = Diagnostic::new(
                Code::E0120,
                self.span(),
                "a query needs a `select` clause naming the result",
            );
            self.emit_recoverable(d);
            Expr { span: self.span(), kind: ExprKind::Error }
        };

        let order = if self.at_kw("order") {
            let ostart = self.span();
            self.bump();
            self.expect(T::By);
            let key = self.parse_expr();
            let descending = self.eat_kw("desc");
            Some(OrderClause { key, descending, span: ostart.to(self.prev_span()) })
        } else {
            None
        };

        let hint = if self.at(T::Hint) {
            let hstart = self.span();
            self.bump();
            let hopen = self.span();
            self.expect(T::LParen);
            let key = self.parse_ident().unwrap_or_else(|| Ident::new("prefer", self.prev_span()));
            self.expect(T::Colon);
            let value = self.parse_ident().unwrap_or_else(|| Ident::new("", self.prev_span()));
            self.expect_close(T::RParen, T::LParen, hopen);
            Some(Hint { key, value, span: hstart.to(self.prev_span()) })
        } else {
            None
        };

        self.expect_close(T::RBrace, T::LBrace, open);
        self.no_struct_lit = saved;
        let span = start.to(self.prev_span());
        Expr {
            span,
            kind: ExprKind::Query(Box::new(QueryExpr {
                froms,
                filter,
                group,
                select,
                order,
                hint,
                span,
            })),
        }
    }

    // -- rules --------------------------------------------------------------------------

    fn parse_rules(&mut self) -> Option<RulesDecl> {
        self.bump(); // rules
        let name = self.parse_ident()?;
        let open = self.span();
        if !self.expect(T::LBrace) {
            return None;
        }
        let mut rels = Vec::new();
        let mut rules = Vec::new();
        loop {
            let comments = self.take_comments();
            if self.at(T::RBrace) || self.at(T::Eof) {
                break;
            }
            let start = self.span();
            if self.at_kw("rel") && self.peek_at(1) == T::Ident {
                self.bump();
                let rname = self.parse_ident()?;
                let popen = self.span();
                self.expect(T::LParen);
                let mut cols = Vec::new();
                loop {
                    if self.at(T::RParen) || self.at(T::Eof) {
                        break;
                    }
                    cols.push(self.parse_type());
                    if !self.eat(T::Comma) {
                        break;
                    }
                }
                self.expect_close(T::RParen, T::LParen, popen);
                self.expect_semi();
                let mut comments = comments;
                let span = start.to(self.prev_span());
                self.take_trailing(&mut comments);
                rels.push(RelDecl { name: rname, cols, span, comments });
            } else {
                let head = self.parse_rule_atom()?;
                self.expect(T::ColonDash);
                let mut body = Vec::new();
                loop {
                    body.push(self.parse_body_atom()?);
                    if !self.eat(T::Comma) {
                        break;
                    }
                }
                self.expect_semi();
                let mut comments = comments;
                let span = start.to(self.prev_span());
                self.take_trailing(&mut comments);
                rules.push(Rule { head, body, span, comments });
            }
            if self.panic {
                break;
            }
        }
        self.expect_close(T::RBrace, T::LBrace, open);
        Some(RulesDecl { name, rels, rules })
    }

    fn parse_rule_atom(&mut self) -> Option<RuleAtom> {
        let start = self.span();
        let name = self.parse_ident()?;
        let open = self.span();
        self.expect(T::LParen);
        let mut terms = Vec::new();
        loop {
            if self.at(T::RParen) || self.at(T::Eof) {
                break;
            }
            terms.push(self.parse_term()?);
            if !self.eat(T::Comma) {
                break;
            }
        }
        self.expect_close(T::RParen, T::LParen, open);
        Some(RuleAtom { name, terms, span: start.to(self.prev_span()) })
    }

    fn parse_body_atom(&mut self) -> Option<BodyAtom> {
        let negated = self.eat(T::Not);
        if self.at(T::Ident) && self.peek_at(1) == T::LParen {
            return Some(BodyAtom::Pred { negated, atom: self.parse_rule_atom()? });
        }
        if negated {
            let d = Diagnostic::new(
                Code::E0121,
                self.span(),
                "`not` applies to a relation atom, e.g. `not blocked(x)`",
            );
            self.emit(d);
            return None;
        }
        let start = self.span();
        let lhs = self.parse_term()?;
        let Some(op) = binop_of(self.peek()).filter(|o| o.is_comparison()) else {
            let d = Diagnostic::new(
                Code::E0121,
                self.span(),
                "expected a relation atom or a comparison in the rule body",
            );
            self.emit(d);
            return None;
        };
        self.bump();
        let rhs = self.parse_term()?;
        Some(BodyAtom::Cmp { lhs, op, rhs, span: start.to(self.prev_span()) })
    }

    fn parse_term(&mut self) -> Option<Term> {
        let start = self.span();
        match self.peek() {
            T::Ident => {
                let t = self.bump();
                Some(Term::Var(Ident::new(self.lexed.text(t), t.span)))
            }
            T::Int => {
                let t = self.bump();
                Some(Term::Int(self.lexed.text(t).replace('_', ""), t.span))
            }
            T::Str => {
                let t = self.bump();
                Some(Term::Str(self.lexed.string_value(t).to_string(), t.span))
            }
            T::True => {
                self.bump();
                Some(Term::Bool(true, start))
            }
            T::False => {
                self.bump();
                Some(Term::Bool(false, start))
            }
            _ => {
                let found = quoted(self.peek());
                let d = Diagnostic::new(
                    Code::E0121,
                    self.span(),
                    format!("expected a variable or literal, found {found}"),
                );
                self.emit(d);
                None
            }
        }
    }

    // -- machine ------------------------------------------------------------------------

    fn parse_machine(&mut self) -> Option<MachineDecl> {
        self.bump(); // machine
        let name = self.parse_ident()?;
        let open = self.span();
        if !self.expect(T::LBrace) {
            return None;
        }
        let states_span = self.span();
        if !self.at(T::States) {
            let found = quoted(self.peek());
            let d = Diagnostic::new(
                Code::E0122,
                self.span(),
                format!("a machine begins with a `states` chain, e.g. `states Draft -> Placed;`, found {found}"),
            );
            self.emit(d);
            return None;
        }
        self.bump();

        let mut state_chains = Vec::new();
        let mut transitions = Vec::new();
        let mut invariants = Vec::new();
        loop {
            let comments = self.take_comments();
            if self.at(T::RBrace) || self.at(T::Eof) {
                break;
            }
            let start = self.span();
            if self.eat(T::Invariant) {
                self.expect(T::Colon);
                let expr = self.parse_expr();
                self.expect_semi();
                let mut comments = comments;
                let span = start.to(self.prev_span());
                self.take_trailing(&mut comments);
                invariants.push(MachineInvariant { expr, span, comments });
                continue;
            }
            let mut links = Vec::new();
            loop {
                let mut alts = Vec::new();
                loop {
                    let Some(s) = self.parse_ident() else { break };
                    alts.push(s);
                    if !self.eat(T::Pipe) {
                        break;
                    }
                }
                if alts.is_empty() {
                    break;
                }
                links.push(alts);
                if !self.eat(T::Arrow) {
                    break;
                }
            }
            if links.is_empty() {
                return None;
            }
            let guard = if self.eat_kw("when") { Some(self.parse_expr()) } else { None };
            self.expect_semi();
            let mut comments = comments;
            let span = start.to(self.prev_span());
            self.take_trailing(&mut comments);
            match guard {
                Some(g) => {
                    if links.len() != 2 || links.iter().any(|l| l.len() != 1) {
                        let d = Diagnostic::new(
                            Code::E0122,
                            span,
                            "a `when` guard applies to a single transition `A -> B when g;`",
                        );
                        self.emit_recoverable(d);
                    } else {
                        transitions.push(Transition {
                            from: links[0][0].clone(),
                            to: links[1][0].clone(),
                            guard: Some(g),
                            span,
                            comments,
                        });
                    }
                }
                None => state_chains.push(StateChain { links, span, comments }),
            }
            if self.panic {
                break;
            }
        }
        self.expect_close(T::RBrace, T::LBrace, open);
        Some(MachineDecl { name, state_chains, transitions, invariants, states_span })
    }
}

enum StmtOrTail {
    Stmt(Stmt),
    Tail(Expr),
    Failed,
}

fn binop_of(k: T) -> Option<BinOp> {
    Some(match k {
        T::Or => BinOp::Or,
        T::And => BinOp::And,
        T::EqEq => BinOp::Eq,
        T::BangEq => BinOp::Ne,
        T::Lt => BinOp::Lt,
        T::Le => BinOp::Le,
        T::Gt => BinOp::Gt,
        T::Ge => BinOp::Ge,
        T::Plus => BinOp::Add,
        T::Minus => BinOp::Sub,
        T::Star => BinOp::Mul,
        T::Slash => BinOp::Div,
        T::Percent => BinOp::Rem,
        _ => return None,
    })
}

/// Assignable places: a local, or a chain of field/index projections from one (§5.2).
fn is_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Path(_) => true,
        ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => is_place(base),
        _ => false,
    }
}

/// Expressions that read as statements without a trailing `;`.
fn is_block_like(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Block(_) | ExprKind::If { .. } | ExprKind::Match { .. })
}
