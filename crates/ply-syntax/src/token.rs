//! Token kinds. The keyword set is exactly the one in §5.1 — everything else that reads
//! like a keyword (`rel`, `order`, `desc`, `when`, `example`, the verification modes) is a
//! contextual keyword recognised by the parser from an ordinary identifier.

use std::fmt;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum TokenKind {
    // literals and names
    Ident,
    Int,
    Str,

    // keywords (§5.1)
    Fn,
    Let,
    Mut,
    Struct,
    Enum,
    Match,
    If,
    Else,
    While,
    For,
    In,
    Return,
    True,
    False,
    Use,
    Requires,
    Ensures,
    Invariant,
    Decreases,
    Verify,
    Uses,
    Query,
    From,
    Where,
    Group,
    By,
    Into,
    Select,
    Rules,
    Machine,
    States,
    Unresolved,
    Dontcare,
    Hint,
    And,
    Or,
    Not,

    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Dot,
    Arrow,     // ->
    FatArrow,  // =>
    ColonDash, // :-
    Pipe,
    Amp,
    Eq,
    EqEq,
    BangEq,
    Bang,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Hash,

    /// Emitted for input the lexer could not classify; already reported.
    Error,
    Eof,
}

impl TokenKind {
    /// How the token is written in source, for `expected ..., found ...` messages.
    pub fn as_str(self) -> &'static str {
        use TokenKind::*;
        match self {
            Ident => "identifier",
            Int => "integer literal",
            Str => "string literal",
            Fn => "fn",
            Let => "let",
            Mut => "mut",
            Struct => "struct",
            Enum => "enum",
            Match => "match",
            If => "if",
            Else => "else",
            While => "while",
            For => "for",
            In => "in",
            Return => "return",
            True => "true",
            False => "false",
            Use => "use",
            Requires => "requires",
            Ensures => "ensures",
            Invariant => "invariant",
            Decreases => "decreases",
            Verify => "verify",
            Uses => "uses",
            Query => "query",
            From => "from",
            Where => "where",
            Group => "group",
            By => "by",
            Into => "into",
            Select => "select",
            Rules => "rules",
            Machine => "machine",
            States => "states",
            Unresolved => "unresolved",
            Dontcare => "dontcare",
            Hint => "hint",
            And => "and",
            Or => "or",
            Not => "not",
            LParen => "(",
            RParen => ")",
            LBrace => "{",
            RBrace => "}",
            LBracket => "[",
            RBracket => "]",
            Comma => ",",
            Semi => ";",
            Colon => ":",
            Dot => ".",
            Arrow => "->",
            FatArrow => "=>",
            ColonDash => ":-",
            Pipe => "|",
            Amp => "&",
            Eq => "=",
            EqEq => "==",
            BangEq => "!=",
            Bang => "!",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Percent => "%",
            Hash => "#",
            Error => "invalid token",
            Eof => "end of file",
        }
    }

    /// True for the fixed keywords of §5.1.
    pub fn is_keyword(self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Fn | Let
                | Mut
                | Struct
                | Enum
                | Match
                | If
                | Else
                | While
                | For
                | In
                | Return
                | True
                | False
                | Use
                | Requires
                | Ensures
                | Invariant
                | Decreases
                | Verify
                | Uses
                | Query
                | From
                | Where
                | Group
                | By
                | Into
                | Select
                | Rules
                | Machine
                | States
                | Unresolved
                | Dontcare
                | Hint
                | And
                | Or
                | Not
        )
    }

    /// Keyword that can begin a top-level item (used for parser recovery).
    pub fn starts_item(self) -> bool {
        use TokenKind::*;
        matches!(self, Fn | Struct | Enum | Use | Rules | Machine)
    }

    pub fn keyword_from_str(s: &str) -> Option<TokenKind> {
        use TokenKind::*;
        Some(match s {
            "fn" => Fn,
            "let" => Let,
            "mut" => Mut,
            "struct" => Struct,
            "enum" => Enum,
            "match" => Match,
            "if" => If,
            "else" => Else,
            "while" => While,
            "for" => For,
            "in" => In,
            "return" => Return,
            "true" => True,
            "false" => False,
            "use" => Use,
            "requires" => Requires,
            "ensures" => Ensures,
            "invariant" => Invariant,
            "decreases" => Decreases,
            "verify" => Verify,
            "uses" => Uses,
            "query" => Query,
            "from" => From,
            "where" => Where,
            "group" => Group,
            "by" => By,
            "into" => Into,
            "select" => Select,
            "rules" => Rules,
            "machine" => Machine,
            "states" => States,
            "unresolved" => Unresolved,
            "dontcare" => Dontcare,
            "hint" => Hint,
            "and" => And,
            "or" => Or,
            "not" => Not,
            _ => return None,
        })
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a token is quoted inside a diagnostic title.
pub fn quoted(kind: TokenKind) -> String {
    match kind {
        TokenKind::Ident | TokenKind::Int | TokenKind::Str | TokenKind::Eof | TokenKind::Error => {
            kind.as_str().to_string()
        }
        _ => format!("`{}`", kind.as_str()),
    }
}
