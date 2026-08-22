//! Property: printing an expression and parsing it back yields the same tree (§15).
//!
//! This is the real test of the formatter's parenthesisation rules — the printer drops the
//! parentheses the author wrote and re-derives them from precedence, so a bug here silently
//! changes what a program means.

use ply_diag::{Diagnostics, Span, SourceMap};
use ply_syntax::ast::*;
use ply_syntax::dump::dump_expr;
use proptest::prelude::*;

fn ident(name: &str) -> Ident {
    Ident::new(name, Span::DUMMY)
}

fn e(kind: ExprKind) -> Expr {
    Expr { kind, span: Span::DUMMY }
}

fn leaf() -> impl Strategy<Value = Expr> {
    prop_oneof![
        (0u32..100).prop_map(|n| e(ExprKind::Int(n.to_string()))),
        any::<bool>().prop_map(|b| e(ExprKind::Bool(b))),
        Just(e(ExprKind::Unit)),
        Just(e(ExprKind::Dontcare)),
        (0u64..4).prop_map(|n| e(ExprKind::Unresolved { id: n })),
        prop::sample::select(vec!["a", "b", "c", "xs"])
            .prop_map(|n| e(ExprKind::Path(ident(n)))),
        prop::sample::select(vec!["", "hi", "a\nb", "q\"q", "\\"])
            .prop_map(|s| e(ExprKind::Str(s.to_string()))),
    ]
}

fn any_binop() -> impl Strategy<Value = BinOp> {
    prop::sample::select(vec![
        BinOp::Or,
        BinOp::And,
        BinOp::Eq,
        BinOp::Ne,
        BinOp::Lt,
        BinOp::Le,
        BinOp::Gt,
        BinOp::Ge,
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Rem,
    ])
}

fn block_of(tail: Expr) -> Block {
    Block { stmts: Vec::new(), tail: Some(Box::new(tail)), trailing_comments: Vec::new(), span: Span::DUMMY }
}

fn expr_strategy() -> impl Strategy<Value = Expr> {
    leaf().prop_recursive(5, 64, 4, |inner| {
        prop_oneof![
            (any_binop(), inner.clone(), inner.clone()).prop_map(|(op, l, r)| e(
                ExprKind::Binary { op, lhs: Box::new(l), rhs: Box::new(r) }
            )),
            (prop::sample::select(vec![UnOp::Not, UnOp::Neg]), inner.clone())
                .prop_map(|(op, x)| e(ExprKind::Unary { op, operand: Box::new(x) })),
            prop::collection::vec(inner.clone(), 0..3).prop_map(|args| e(ExprKind::Call {
                callee: Box::new(e(ExprKind::Path(ident("f")))),
                type_args: Vec::new(),
                args,
            })),
            (inner.clone(), prop::collection::vec(inner.clone(), 0..2)).prop_map(|(r, args)| e(
                ExprKind::MethodCall {
                    receiver: Box::new(r),
                    method: ident("m"),
                    type_args: Vec::new(),
                    args,
                }
            )),
            inner.clone().prop_map(|x| e(ExprKind::Field { base: Box::new(x), name: ident("k") })),
            (inner.clone(), inner.clone()).prop_map(|(b, i)| e(ExprKind::Index {
                base: Box::new(b),
                index: Box::new(i)
            })),
            prop::collection::vec(inner.clone(), 0..3).prop_map(|xs| e(ExprKind::List(xs))),
            prop::collection::vec(inner.clone(), 0..2).prop_map(|vs| e(ExprKind::StructLit {
                name: ident("S"),
                fields: vs
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| FieldInit {
                        name: ident(if i == 0 { "x" } else { "y" }),
                        value: v,
                        span: Span::DUMMY,
                        comments: Comments::default(),
                    })
                    .collect(),
            })),
            (inner.clone(), inner.clone(), inner.clone()).prop_map(|(c, t, f)| e(ExprKind::If {
                cond: Box::new(c),
                then: Box::new(block_of(t)),
                else_: Some(Box::new(e(ExprKind::Block(Box::new(block_of(f)))))),
            })),
        ]
    })
}

fn reparse(src: &str) -> (Expr, Diagnostics) {
    let mut sm = SourceMap::new();
    let file = sm.add("prop.ply", src);
    let mut d = Diagnostics::new();
    let parsed = ply_syntax::parse_expression(file, &sm.source(file), &mut d);
    (parsed, d)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn printing_then_parsing_preserves_the_expression(expr in expr_strategy()) {
        let printed = ply_syntax::fmt::format_expr(&expr);
        let (back, diags) = reparse(&printed);
        prop_assert!(
            diags.is_empty(),
            "printed form does not parse: {printed}\n{:?}",
            diags.iter().map(|d| (d.code, d.title.clone())).collect::<Vec<_>>()
        );
        prop_assert_eq!(dump_expr(&expr), dump_expr(&back), "printed as: {}", printed);
    }

    #[test]
    fn printing_is_idempotent(expr in expr_strategy()) {
        let once = ply_syntax::fmt::format_expr(&expr);
        let (back, _) = reparse(&once);
        prop_assert_eq!(&once, &ply_syntax::fmt::format_expr(&back));
    }
}
