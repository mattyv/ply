//! The Ply AST (§5.2). This is a *concrete-ish* tree: it keeps comments and enough spans to
//! drive the canonical formatter, but redundant parentheses are dropped — the printer
//! re-derives them from precedence, which is what makes `ply fmt` canonical.

use ply_diag::Span;

// ---------------------------------------------------------------------------------------
// Shared leaves
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Ident {
        Ident { name: name.into(), span }
    }

    /// Types are `UpperCamel`, values are `snake_case` — enforced by [`crate::naming`].
    pub fn looks_like_type(&self) -> bool {
        self.name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
    }
}

/// A comment attached to a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub text: String,
    pub span: Span,
    pub block: bool,
    /// Blank lines between the previous node/comment and this one, capped at 1 by the
    /// formatter.
    pub blank_before: bool,
}

/// Comments owned by a node: those written above it, and one written after it on the same
/// line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comments {
    pub leading: Vec<Comment>,
    pub trailing: Option<Comment>,
    /// A blank line separated this node from the previous one.
    pub blank_before: bool,
}

impl Comments {
    pub fn is_empty(&self) -> bool {
        self.leading.is_empty() && self.trailing.is_none()
    }
}

// ---------------------------------------------------------------------------------------
// Files and items
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub items: Vec<Item>,
    /// Comments after the last item.
    pub trailing_comments: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: ItemKind,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Use(UseDecl),
    Fn(Box<FnDecl>),
    Struct(StructDecl),
    Enum(EnumDecl),
    Rules(RulesDecl),
    Machine(MachineDecl),
    Example(ExampleDecl),
}

impl ItemKind {
    pub fn name(&self) -> &Ident {
        match self {
            ItemKind::Use(u) => &u.name,
            ItemKind::Fn(f) => &f.name,
            ItemKind::Struct(s) => &s.name,
            ItemKind::Enum(e) => &e.name,
            ItemKind::Rules(r) => &r.name,
            ItemKind::Machine(m) => &m.name,
            ItemKind::Example(e) => &e.target,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseDecl {
    pub name: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDecl {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: Ident,
    pub ty: Type,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumDecl {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub variants: Vec<Variant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub name: Ident,
    pub payload: Vec<Type>,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnDecl {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub params: Vec<Param>,
    pub ret: Type,
    pub uses: Option<UsesClause>,
    pub contracts: Vec<Contract>,
    pub verify: Option<VerifyClause>,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: Ident,
    pub mode: ParamMode,
    pub ty: Type,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ParamMode {
    /// `x: T` — passed by value (moved unless the type is Copy).
    Owned,
    /// `x: &T`
    Ref,
    /// `x: &mut T`
    RefMut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsesClause {
    pub caps: Vec<Capability>,
    pub span: Span,
}

/// The fixed capability set of v0 (§5.4).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cap {
    IoRead,
    IoWrite,
    DbRead,
    DbWrite,
    Rand,
    Time,
}

impl Cap {
    pub const ALL: &'static [Cap] =
        &[Cap::IoRead, Cap::IoWrite, Cap::DbRead, Cap::DbWrite, Cap::Rand, Cap::Time];

    pub fn as_str(self) -> &'static str {
        match self {
            Cap::IoRead => "io.read",
            Cap::IoWrite => "io.write",
            Cap::DbRead => "db.read",
            Cap::DbWrite => "db.write",
            Cap::Rand => "rand",
            Cap::Time => "time",
        }
    }

    pub fn from_str(s: &str) -> Option<Cap> {
        Cap::ALL.iter().copied().find(|c| c.as_str() == s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    pub cap: Cap,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Contract {
    Requires { expr: Expr, span: Span, comments: Comments },
    Ensures { binder: Ident, expr: Expr, span: Span, comments: Comments },
    Decreases { expr: Expr, span: Span, comments: Comments },
}

impl Contract {
    pub fn span(&self) -> Span {
        match self {
            Contract::Requires { span, .. }
            | Contract::Ensures { span, .. }
            | Contract::Decreases { span, .. } => *span,
        }
    }

    pub fn comments(&self) -> &Comments {
        match self {
            Contract::Requires { comments, .. }
            | Contract::Ensures { comments, .. }
            | Contract::Decreases { comments, .. } => comments,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyClause {
    pub mode: VerifyMode,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VerifyMode {
    Test,
    Fuzz { runs: Option<u64> },
    Bounded { depth: Option<u64> },
    Induct { k: Option<u64> },
    Prove,
}

impl VerifyMode {
    /// Defaults from §5.6.
    pub const DEFAULT_RUNS: u64 = 256;
    pub const DEFAULT_DEPTH: u64 = 2;
    pub const DEFAULT_K: u64 = 2;

    pub fn keyword(self) -> &'static str {
        match self {
            VerifyMode::Test => "test",
            VerifyMode::Fuzz { .. } => "fuzz",
            VerifyMode::Bounded { .. } => "bounded",
            VerifyMode::Induct { .. } => "induct",
            VerifyMode::Prove => "prove",
        }
    }

    /// `bounded(depth: 2)`-style rendering, with defaults made explicit.
    pub fn describe(self) -> String {
        match self {
            VerifyMode::Test => "test".to_string(),
            VerifyMode::Fuzz { runs } => {
                format!("fuzz(runs: {})", runs.unwrap_or(Self::DEFAULT_RUNS))
            }
            VerifyMode::Bounded { depth } => {
                format!("bounded(depth: {})", depth.unwrap_or(Self::DEFAULT_DEPTH))
            }
            VerifyMode::Induct { k } => format!("induct(k: {})", k.unwrap_or(Self::DEFAULT_K)),
            VerifyMode::Prove => "prove".to_string(),
        }
    }
}

/// `example add(1, 2) == 3;` (§9.2)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExampleDecl {
    pub target: Ident,
    pub args: Vec<Expr>,
    pub expected: Expr,
}

// ---------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Unit,
    Named { name: Ident, args: Vec<Type> },
    /// Only produced by error recovery.
    Error,
}

// ---------------------------------------------------------------------------------------
// Statements and blocks
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// The trailing expression, if the block ends without a `;`.
    pub tail: Option<Box<Expr>>,
    /// Comments between the last statement and the closing brace.
    pub trailing_comments: Vec<Comment>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StmtKind {
    Let { mutable: bool, name: Ident, ty: Option<Type>, init: Expr },
    Assign { target: Expr, value: Expr },
    While { cond: Expr, invariants: Vec<Expr>, decreases: Option<Expr>, body: Block },
    For { var: Ident, iter: Expr, body: Block },
    Return(Option<Expr>),
    Expr(Expr),
}

// ---------------------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Int(String),
    Str(String),
    Bool(bool),
    Unit,
    Path(Ident),
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Unary { op: UnOp, operand: Box<Expr> },
    Call { callee: Box<Expr>, type_args: Vec<Type>, args: Vec<Expr> },
    /// `x.f(a)` — the method-call sugar of §5.2, resolved during HIR lowering.
    MethodCall { receiver: Box<Expr>, method: Ident, type_args: Vec<Type>, args: Vec<Expr> },
    Field { base: Box<Expr>, name: Ident },
    Index { base: Box<Expr>, index: Box<Expr> },
    Block(Box<Block>),
    If { cond: Box<Expr>, then: Box<Block>, else_: Option<Box<Expr>> },
    Match { scrutinee: Box<Expr>, arms: Vec<Arm> },
    List(Vec<Expr>),
    StructLit { name: Ident, fields: Vec<FieldInit> },
    Query(Box<QueryExpr>),
    /// `.field` — the selector argument of a query aggregate, e.g. `sum(g, .total)` (§5.8).
    FieldRef(Ident),
    Unresolved { id: u64 },
    Dontcare,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldInit {
    pub name: Ident,
    pub value: Expr,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinOp {
    pub fn as_str(self) -> &'static str {
        match self {
            BinOp::Or => "or",
            BinOp::And => "and",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
        }
    }

    /// Binding power: higher binds tighter (§5.2).
    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne => 3,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
            BinOp::Add | BinOp::Sub => 5,
            BinOp::Mul | BinOp::Div | BinOp::Rem => 6,
        }
    }

    /// Comparisons and equality do not chain: `a < b < c` is a parse error.
    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

impl UnOp {
    pub fn as_str(self) -> &'static str {
        match self {
            UnOp::Not => "not",
            UnOp::Neg => "-",
        }
    }
}

// ---------------------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Wildcard,
    Binding(Ident),
    Int(String),
    Str(String),
    Bool(bool),
    Unit,
    /// `Some(x)`, `None`, `Point(a, b)`.
    Variant { name: Ident, args: Option<Vec<Pattern>> },
    /// `{ x, y: p }`
    Struct { fields: Vec<FieldPattern> },
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldPattern {
    pub name: Ident,
    /// `None` for the shorthand `{ x }`.
    pub pattern: Option<Pattern>,
    pub span: Span,
}

// ---------------------------------------------------------------------------------------
// query
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryExpr {
    pub froms: Vec<FromClause>,
    pub filter: Option<Expr>,
    pub group: Option<GroupClause>,
    pub select: Expr,
    pub order: Option<OrderClause>,
    pub hint: Option<Hint>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FromClause {
    pub var: Ident,
    pub source: Expr,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupClause {
    /// `group ROW by KEY into GROUP`
    pub row: Ident,
    pub key: Expr,
    pub binding: Ident,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderClause {
    pub key: Expr,
    pub descending: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub key: Ident,
    pub value: Ident,
    pub span: Span,
}

// ---------------------------------------------------------------------------------------
// rules
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulesDecl {
    pub name: Ident,
    pub rels: Vec<RelDecl>,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelDecl {
    pub name: Ident,
    pub cols: Vec<Type>,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub head: RuleAtom,
    pub body: Vec<BodyAtom>,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleAtom {
    pub name: Ident,
    pub terms: Vec<Term>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyAtom {
    Pred { negated: bool, atom: RuleAtom },
    Cmp { lhs: Term, op: BinOp, rhs: Term, span: Span },
}

impl BodyAtom {
    pub fn span(&self) -> Span {
        match self {
            BodyAtom::Pred { atom, .. } => atom.span,
            BodyAtom::Cmp { span, .. } => *span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Var(Ident),
    Int(String, Span),
    Str(String, Span),
    Bool(bool, Span),
}

impl Term {
    pub fn span(&self) -> Span {
        match self {
            Term::Var(i) => i.span,
            Term::Int(_, s) | Term::Str(_, s) | Term::Bool(_, s) => *s,
        }
    }
}

// ---------------------------------------------------------------------------------------
// machine
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineDecl {
    pub name: Ident,
    pub state_chains: Vec<StateChain>,
    pub transitions: Vec<Transition>,
    pub invariants: Vec<MachineInvariant>,
    pub states_span: Span,
}

/// `Draft -> Placed -> Filled | Cancelled`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateChain {
    /// Each link is a set of alternatives separated by `|`.
    pub links: Vec<Vec<Ident>>,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub from: Ident,
    pub to: Ident,
    pub guard: Option<Expr>,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineInvariant {
    pub expr: Expr,
    pub span: Span,
    pub comments: Comments,
}
