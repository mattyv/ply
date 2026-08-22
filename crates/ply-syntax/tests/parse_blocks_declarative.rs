//! query / rules / machine blocks (§5.8, §5.9, §5.10).

mod util;
use util::{dump_expr, dump_src, errors, parse_ok};

#[test]
fn minimal_query() {
    assert_eq!(
        dump_expr("query { from o in orders select o }"),
        "(query (from o orders) (select o))"
    );
}

#[test]
fn query_with_every_clause() {
    let src = "query { from o in orders, from c in customers \
               where o.customer_id == c.id and o.total > 100 \
               group o by o.customer_id into g \
               select Row { id: g.key, total: sum(g, .total) } \
               order by g.key desc \
               hint(prefer: hash_join) }";
    assert_eq!(
        dump_expr(src),
        "(query (from o orders) (from c customers) \
         (where (and (== (field o customer_id) (field c id)) (> (field o total) 100))) \
         (group o (field o customer_id) g) \
         (select (struct-lit Row (id (field g key)) (total (call sum g (fieldref total))))) \
         (order (field g key) desc) (hint prefer hash_join))"
    );
}

#[test]
fn query_order_without_desc_is_ascending() {
    assert_eq!(
        dump_expr("query { from o in os select o order by o.k }"),
        "(query (from o os) (select o) (order (field o k) asc))"
    );
}

#[test]
fn query_missing_select_is_e0120() {
    assert_eq!(errors("fn f() -> Int { query { from o in os } }"), vec!["E0120"]);
}

#[test]
fn rules_block() {
    let src = "rules Access {
        rel parent(String, String);
        rel blocked(String);
        ancestor(x, y) :- parent(x, y);
        ancestor(x, z) :- parent(x, y), ancestor(y, z);
        allowed(x) :- ancestor(x, \"root\"), not blocked(x), x != \"nobody\";
    }";
    assert_eq!(
        dump_src(src),
        "(file (rules Access \
         (rel parent String String) (rel blocked String) \
         (rule (atom ancestor x y) (atom parent x y)) \
         (rule (atom ancestor x z) (atom parent x y) (atom ancestor y z)) \
         (rule (atom allowed x) (atom ancestor x \"root\") (not (atom blocked x)) \
         (cmp != x \"nobody\"))))"
    );
}

#[test]
fn rules_require_semicolons() {
    assert_eq!(
        errors("rules R { rel p(Int); q(x) :- p(x) }"),
        vec!["E0110"]
    );
}

#[test]
fn machine_block() {
    let src = "machine Order {
        states Draft -> Placed -> Filled | Cancelled;
        Placed -> Cancelled when !ev.partially_filled;
        invariant: true;
    }";
    assert_eq!(
        dump_src(src),
        "(file (machine Order \
         (chain (Draft) (Placed) (Filled Cancelled)) \
         (transition Placed Cancelled (not (field ev partially_filled))) \
         (invariant true)))"
    );
}

#[test]
fn machine_with_several_guardless_chains() {
    let src = "machine M { states A -> B; B -> C; C -> A; }";
    assert_eq!(
        dump_src(src),
        "(file (machine M (chain (A) (B)) (chain (B) (C)) (chain (C) (A))))"
    );
}

#[test]
fn machine_guard_must_apply_to_a_single_transition() {
    assert_eq!(errors("machine M { states A -> B -> C when g; }"), vec!["E0122"]);
}

#[test]
fn machine_requires_a_states_clause() {
    assert_eq!(errors("machine M { A -> B; }"), vec!["E0122"]);
}

#[test]
fn declarative_blocks_round_trip_through_a_function_body() {
    parse_ok("fn f(os: List[Order]) -> List[Order] { query { from o in os where o.n > 0 select o } }");
}
