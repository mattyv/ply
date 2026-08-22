//! The lexer (§5.1). Produces a token vector with comment trivia attached to the following
//! token, which is what the canonical formatter needs to preserve comments.

use crate::token::TokenKind;
use logos::Logos;
use ply_diag::{Code, Diagnostic, Diagnostics, FileId, Fix, Span};
use std::sync::Arc;

#[derive(Logos, Debug, PartialEq)]
#[logos(skip r"[ \t\r\n\f\u{0B}]+")]
enum Raw {
    #[regex(r"//[^\n]*", allow_greedy = true)]
    LineComment,
    #[regex(r"/\*[^*]*\*+([^/*][^*]*\*+)*/")]
    BlockComment,
    /// `/*` with no `*/` after it: swallow the rest of the file so we report once.
    #[token("/*", |lex| { lex.bump(lex.remainder().len()); })]
    UnterminatedBlockComment,

    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident,
    #[regex(r"[0-9](_?[0-9])*")]
    Int,
    #[regex(r#""([^"\\\n]|\\.)*""#)]
    Str,

    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(",")]
    Comma,
    #[token(";")]
    Semi,
    #[token(":-")]
    ColonDash,
    #[token(":")]
    Colon,
    #[token(".")]
    Dot,
    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,
    #[token("|")]
    Pipe,
    #[token("&")]
    Amp,
    #[token("==")]
    EqEq,
    #[token("=")]
    Eq,
    #[token("!=")]
    BangEq,
    #[token("!")]
    Bang,
    #[token("<=")]
    Le,
    #[token("<")]
    Lt,
    #[token(">=")]
    Ge,
    #[token(">")]
    Gt,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("#")]
    Hash,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TriviaKind {
    Line,
    Block,
}

#[derive(Clone, Debug)]
pub struct Trivia {
    pub kind: TriviaKind,
    pub span: Span,
    /// Newlines between the preceding token/trivia and this comment. `0` means the comment
    /// sits on the same line as whatever came before it (a trailing comment).
    pub newlines_before: u32,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    /// Newlines between the previous token (or its trailing trivia) and this token.
    pub newlines_before: u32,
    /// Index range into [`Lexed::trivia`].
    pub leading_start: u32,
    pub leading_len: u32,
    /// For [`TokenKind::Str`], the index of the decoded value in [`Lexed::strings`].
    pub aux: u32,
}

impl Token {
    pub fn leading(&self) -> std::ops::Range<usize> {
        let s = self.leading_start as usize;
        s..s + self.leading_len as usize
    }
}

/// The result of lexing one file.
#[derive(Clone, Debug)]
pub struct Lexed {
    pub file: FileId,
    pub source: Arc<str>,
    pub tokens: Vec<Token>,
    pub trivia: Vec<Trivia>,
    /// Decoded string-literal values, indexed by `Token::aux`.
    pub strings: Vec<String>,
}

impl Lexed {
    /// Source text of a token, verbatim.
    pub fn text(&self, tok: &Token) -> &str {
        let start = tok.span.start as usize;
        let end = (tok.span.end as usize).min(self.source.len());
        &self.source[start.min(end)..end]
    }

    pub fn trivia_text(&self, t: &Trivia) -> &str {
        &self.source[t.span.start as usize..t.span.end as usize]
    }

    pub fn string_value(&self, tok: &Token) -> &str {
        self.strings.get(tok.aux as usize).map(String::as_str).unwrap_or("")
    }
}

fn newlines_in(s: &str) -> u32 {
    s.bytes().filter(|&b| b == b'\n').count() as u32
}

/// Lex `source`. Errors are recorded in `diags`; lexing always produces a token stream so
/// the parser can still make progress.
pub fn lex(file: FileId, source: &Arc<str>, diags: &mut Diagnostics) -> Lexed {
    let mut tokens = Vec::new();
    let mut trivia = Vec::new();
    let mut strings = Vec::new();

    let mut lx = Raw::lexer(source.as_ref());
    let mut prev_end = 0usize;
    let mut pending_trivia_start = trivia.len() as u32;
    // Newlines seen since the previous token, before any pending trivia is flushed.
    let mut newlines_for_token: u32 = 0;
    let mut first_gap_done = false;

    while let Some(res) = lx.next() {
        let range = lx.span();
        let gap_newlines = newlines_in(&source[prev_end..range.start]);
        prev_end = range.end;

        match res {
            Ok(Raw::LineComment) | Ok(Raw::BlockComment) => {
                let kind = if matches!(res, Ok(Raw::LineComment)) {
                    TriviaKind::Line
                } else {
                    TriviaKind::Block
                };
                trivia.push(Trivia {
                    kind,
                    span: Span::new(file, range.start as u32, range.end as u32),
                    newlines_before: gap_newlines,
                });
                if !first_gap_done {
                    newlines_for_token = gap_newlines;
                    first_gap_done = true;
                }
                continue;
            }
            _ => {}
        }

        let span = Span::new(file, range.start as u32, range.end as u32);
        let newlines_before = if first_gap_done { newlines_for_token } else { gap_newlines };
        first_gap_done = false;
        newlines_for_token = 0;

        let mut aux = 0u32;
        let kind = match res {
            Ok(Raw::UnterminatedBlockComment) => {
                diags.push(
                    Diagnostic::new(Code::E0105, span, "unterminated block comment")
                        .with_fix(Fix::insert_after("close the comment", span, " */")),
                );
                TokenKind::Error
            }
            Ok(Raw::Ident) => {
                let text = &source[range.clone()];
                TokenKind::keyword_from_str(text).unwrap_or(TokenKind::Ident)
            }
            Ok(Raw::Int) => TokenKind::Int,
            Ok(Raw::Str) => {
                let raw = &source[range.clone()];
                match decode_string(raw, span) {
                    Ok(v) => {
                        aux = strings.len() as u32;
                        strings.push(v);
                    }
                    Err(d) => {
                        diags.push(d);
                        aux = strings.len() as u32;
                        strings.push(String::new());
                    }
                }
                TokenKind::Str
            }
            Ok(raw) => punct_kind(raw),
            Err(()) => {
                let text = &source[range.clone()];
                diags.push(lex_error(file, span, text));
                TokenKind::Error
            }
        };

        tokens.push(Token {
            kind,
            span,
            newlines_before,
            leading_start: pending_trivia_start,
            leading_len: trivia.len() as u32 - pending_trivia_start,
            aux,
        });
        pending_trivia_start = trivia.len() as u32;
    }

    let end = source.len() as u32;
    let gap_newlines = newlines_in(&source[prev_end..]);
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(file, end, end),
        newlines_before: if first_gap_done { newlines_for_token } else { gap_newlines },
        leading_start: pending_trivia_start,
        leading_len: trivia.len() as u32 - pending_trivia_start,
        aux: 0,
    });

    let lexed = Lexed { file, source: source.clone(), tokens, trivia, strings };
    check_adjacent_numbers(&lexed, diags);
    lexed
}

fn punct_kind(raw: Raw) -> TokenKind {
    match raw {
        Raw::LParen => TokenKind::LParen,
        Raw::RParen => TokenKind::RParen,
        Raw::LBrace => TokenKind::LBrace,
        Raw::RBrace => TokenKind::RBrace,
        Raw::LBracket => TokenKind::LBracket,
        Raw::RBracket => TokenKind::RBracket,
        Raw::Comma => TokenKind::Comma,
        Raw::Semi => TokenKind::Semi,
        Raw::Colon => TokenKind::Colon,
        Raw::ColonDash => TokenKind::ColonDash,
        Raw::Dot => TokenKind::Dot,
        Raw::Arrow => TokenKind::Arrow,
        Raw::FatArrow => TokenKind::FatArrow,
        Raw::Pipe => TokenKind::Pipe,
        Raw::Amp => TokenKind::Amp,
        Raw::Eq => TokenKind::Eq,
        Raw::EqEq => TokenKind::EqEq,
        Raw::BangEq => TokenKind::BangEq,
        Raw::Bang => TokenKind::Bang,
        Raw::Lt => TokenKind::Lt,
        Raw::Le => TokenKind::Le,
        Raw::Gt => TokenKind::Gt,
        Raw::Ge => TokenKind::Ge,
        Raw::Plus => TokenKind::Plus,
        Raw::Minus => TokenKind::Minus,
        Raw::Star => TokenKind::Star,
        Raw::Slash => TokenKind::Slash,
        Raw::Percent => TokenKind::Percent,
        Raw::Hash => TokenKind::Hash,
        Raw::Ident
        | Raw::Int
        | Raw::Str
        | Raw::LineComment
        | Raw::BlockComment
        | Raw::UnterminatedBlockComment => TokenKind::Error,
    }
}

/// `123abc` lexes as two tokens; that is never what the author meant.
fn check_adjacent_numbers(lexed: &Lexed, diags: &mut Diagnostics) {
    for pair in lexed.tokens.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.kind == TokenKind::Int
            && matches!(b.kind, TokenKind::Int | TokenKind::Ident)
            && a.span.end == b.span.start
        {
            let span = a.span.to(b.span);
            diags.push(Diagnostic::new(
                Code::E0106,
                span,
                format!("`{}` is not a valid integer literal", &lexed.source[span.start as usize..span.end as usize]),
            ));
        }
    }
}

fn lex_error(file: FileId, span: Span, text: &str) -> Diagnostic {
    let first = text.chars().next().unwrap_or('\u{fffd}');
    match first {
        '"' => Diagnostic::new(Code::E0103, span, "unterminated string literal")
            .with_fix(Fix::insert_after("close the string", span, "\"")),
        c => {
            let mut d = Diagnostic::new(
                Code::E0102,
                span,
                format!("unexpected character `{}`", c.escape_debug()),
            );
            if let Some(sub) = suggested_replacement(c) {
                d = d.with_fix(Fix::replace(format!("use `{sub}`"), span, sub));
            }
            let _ = file;
            d
        }
    }
}

fn suggested_replacement(c: char) -> Option<&'static str> {
    Some(match c {
        '&' => "and",
        '~' => "not",
        '^' => "*",
        '\'' => "\"",
        '@' | '$' | '?' | '\\' => return None,
        _ => return None,
    })
}

/// Decode a `"..."` literal. `raw` includes the surrounding quotes.
fn decode_string(raw: &str, span: Span) -> Result<String, Diagnostic> {
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.char_indices();
    while let Some((i, c)) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let bad = |len: usize, msg: &str| {
            let start = span.start + 1 + i as u32;
            Diagnostic::new(
                Code::E0104,
                Span::new(span.file, start, start + len as u32),
                msg.to_string(),
            )
        };
        let Some((_, esc)) = chars.next() else {
            return Err(bad(1, "trailing `\\` in string literal"));
        };
        match esc {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'u' => {
                let Some((_, '{')) = chars.next() else {
                    return Err(bad(2, "`\\u` must be followed by `{...}`, e.g. `\\u{1F600}`"));
                };
                let mut hex = String::new();
                let mut closed = false;
                for (_, h) in chars.by_ref() {
                    if h == '}' {
                        closed = true;
                        break;
                    }
                    hex.push(h);
                }
                if !closed || hex.is_empty() || hex.len() > 6 {
                    return Err(bad(hex.len() + 3, "malformed `\\u{...}` escape"));
                }
                let Ok(n) = u32::from_str_radix(&hex, 16) else {
                    return Err(bad(hex.len() + 3, "`\\u{...}` needs hexadecimal digits"));
                };
                let Some(ch) = char::from_u32(n) else {
                    return Err(bad(hex.len() + 3, format!("`\\u{{{hex}}}` is not a character").as_str()));
                };
                out.push(ch);
            }
            other => {
                return Err(bad(
                    2,
                    format!(
                        "unknown escape `\\{}`; Ply supports \\n \\t \\\\ \\\" and \\u{{...}}",
                        other.escape_debug()
                    )
                    .as_str(),
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex_str(src: &str) -> (Lexed, Diagnostics) {
        let mut d = Diagnostics::new();
        let arc: Arc<str> = Arc::from(src);
        let l = lex(FileId(0), &arc, &mut d);
        (l, d)
    }

    fn kinds(src: &str) -> Vec<TokenKind> {
        let (l, d) = lex_str(src);
        assert!(d.is_empty(), "unexpected diagnostics: {:?}", d.iter().map(|x| x.code).collect::<Vec<_>>());
        l.tokens.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn keywords_and_idents() {
        use TokenKind::*;
        assert_eq!(kinds("fn main x"), vec![Fn, Ident, Ident, Eof]);
        // contextual keywords stay identifiers
        assert_eq!(kinds("rel order desc when example prove"), vec![Ident; 6].into_iter().chain([Eof]).collect::<Vec<_>>());
    }

    #[test]
    fn multichar_punctuation_wins() {
        use TokenKind::*;
        assert_eq!(kinds("-> => :- == != <= >= : - ="), vec![
            Arrow, FatArrow, ColonDash, EqEq, BangEq, Le, Ge, Colon, Minus, Eq, Eof
        ]);
    }

    #[test]
    fn integers_allow_single_underscores() {
        assert_eq!(kinds("1_000_000"), vec![TokenKind::Int, TokenKind::Eof]);
        let (_, d) = lex_str("123abc");
        assert_eq!(d.iter().next().unwrap().code, Code::E0106);
    }

    #[test]
    fn strings_decode_escapes() {
        let (l, d) = lex_str(r#""a\nb\u{41}\"""#);
        assert!(d.is_empty());
        assert_eq!(l.string_value(&l.tokens[0]), "a\nbA\"");
    }

    #[test]
    fn bad_escape_is_e0104() {
        let (_, d) = lex_str(r#""a\qb""#);
        assert_eq!(d.iter().next().unwrap().code, Code::E0104);
    }

    #[test]
    fn unterminated_string_is_e0103() {
        let (_, d) = lex_str("\"abc\n");
        assert_eq!(d.iter().next().unwrap().code, Code::E0103);
    }

    #[test]
    fn unterminated_block_comment_is_e0105() {
        let (_, d) = lex_str("/* hi ");
        assert_eq!(d.iter().next().unwrap().code, Code::E0105);
    }

    #[test]
    fn comments_attach_as_leading_trivia() {
        let (l, d) = lex_str("// header\n\n/* b */ fn");
        assert!(d.is_empty());
        let fn_tok = &l.tokens[0];
        assert_eq!(fn_tok.kind, TokenKind::Fn);
        assert_eq!(fn_tok.leading_len, 2);
        assert_eq!(l.trivia[0].kind, TriviaKind::Line);
        assert_eq!(l.trivia[1].newlines_before, 2);
        // the `fn` token itself is on the same line as the block comment
        assert_eq!(fn_tok.newlines_before, 0);
    }

    #[test]
    fn block_comments_do_not_nest() {
        let (l, d) = lex_str("/* a /* b */ c");
        assert!(d.is_empty(), "{:?}", d.iter().map(|x| x.code).collect::<Vec<_>>());
        assert_eq!(l.trivia.len(), 1);
        assert_eq!(l.tokens[0].kind, TokenKind::Ident);
    }

    #[test]
    fn eof_carries_trailing_comments() {
        let (l, _) = lex_str("fn\n// tail\n");
        let eof = l.tokens.last().unwrap();
        assert_eq!(eof.kind, TokenKind::Eof);
        assert_eq!(eof.leading_len, 1);
    }
}
