//! Layered top-to-bottom placement for the top-level component graph.
//!
//! Concepts pinched from archi-techture's dagre-based layout
//! (`src/view/layout/dagre-layout.ts`) — NOT its code: that renderer drives a
//! real dagre graph library over potentially large graphs; ours are tiny
//! (single-digit to low-double-digit components), so a from-scratch
//! longest-path layering is enough:
//!
//! - Rank each node by the longest path over edge direction ("dagre" calls
//!   this network-simplex-refined longest-path layering; we stop at plain
//!   longest-path, which is what dagre itself falls back to and is exact for
//!   graphs this small).
//! - Cycles (e.g. a component pair with edges pointing both ways, such as a
//!   consumer that both reads a ring buffer and is written back into it) are
//!   broken by reversing DFS back-edges before ranking — the standard
//!   Sugiyama/dagre "make acyclic" step — so ranking always terminates and
//!   never revisits the same node twice.
//! - Nodes in the same rank sit in one row, left to right in declaration
//!   order; rows stack top to bottom. Fixed `ranksep`/`nodesep` gaps (the
//!   dagre-layout.ts constants were 52/32 for its node sizes; ours are wider
//!   because our boxes are taller and carry edge labels between ranks).

use indexmap::IndexMap;
use std::collections::{HashSet, VecDeque};

/// Reverses DFS back-edges so the edge set becomes acyclic, preserving every
/// forward/cross edge as-is. A back edge `u -> v` (v currently on the DFS
/// stack) is recorded as `v -> u` instead: this is the direction that was
/// already established by the time `u` was reached, so it costs the ranking
/// nothing and breaks the cycle.
fn acyclic_edges(names: &[String], edges: &[(String, String)]) -> Vec<(String, String)> {
    let mut adj: IndexMap<&str, Vec<&str>> = IndexMap::new();
    for n in names {
        adj.entry(n.as_str()).or_default();
    }
    let declared: HashSet<&str> = names.iter().map(|n| n.as_str()).collect();
    for (a, b) in edges {
        if a == b {
            continue; // self-loops carry no ranking information
        }
        if !declared.contains(a.as_str()) || !declared.contains(b.as_str()) {
            continue; // an edge to a node nobody declared has nothing to rank
        }
        adj.entry(a.as_str()).or_default().push(b.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut on_stack: HashSet<&str> = HashSet::new();
    let mut out: Vec<(String, String)> = Vec::new();

    fn visit<'a>(
        u: &'a str,
        adj: &IndexMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        on_stack: &mut HashSet<&'a str>,
        out: &mut Vec<(String, String)>,
    ) {
        visited.insert(u);
        on_stack.insert(u);
        if let Some(succs) = adj.get(u) {
            for &v in succs {
                if on_stack.contains(v) {
                    out.push((v.to_string(), u.to_string())); // back edge, reversed
                } else {
                    out.push((u.to_string(), v.to_string()));
                    if !visited.contains(v) {
                        visit(v, adj, visited, on_stack, out);
                    }
                }
            }
        }
        on_stack.remove(u);
    }

    for n in names {
        if !visited.contains(n.as_str()) {
            visit(n.as_str(), &adj, &mut visited, &mut on_stack, &mut out);
        }
    }
    out
}

/// Longest-path layering: rank 0 is every source (no incoming edge in the
/// acyclic edge set); every other node's rank is one more than the longest
/// chain of predecessors reaching it. Isolated nodes (no edges at all) land
/// at rank 0, same as any other source.
pub fn assign_ranks(names: &[String], edges: &[(String, String)]) -> IndexMap<String, usize> {
    let acyclic = acyclic_edges(names, edges);

    let mut succ: IndexMap<&str, Vec<&str>> = IndexMap::new();
    let mut indeg: IndexMap<&str, usize> = IndexMap::new();
    for n in names {
        succ.entry(n.as_str()).or_default();
        indeg.entry(n.as_str()).or_insert(0);
    }
    for (a, b) in &acyclic {
        succ.entry(a.as_str()).or_default().push(b.as_str());
        *indeg.entry(b.as_str()).or_insert(0) += 1;
    }

    let mut rank: IndexMap<&str, usize> = names.iter().map(|n| (n.as_str(), 0)).collect();
    let mut remaining = indeg.clone();
    let mut queue: VecDeque<&str> = names
        .iter()
        .filter(|n| indeg[n.as_str()] == 0)
        .map(|n| n.as_str())
        .collect();

    while let Some(u) = queue.pop_front() {
        let ru = rank[u];
        if let Some(succs) = succ.get(u) {
            for &v in succs {
                if ru + 1 > rank[v] {
                    rank.insert(v, ru + 1);
                }
                let d = remaining.get_mut(v).expect("v was seeded into indeg above");
                *d -= 1;
                if *d == 0 {
                    queue.push_back(v);
                }
            }
        }
    }

    names
        .iter()
        .map(|n| (n.clone(), rank[n.as_str()]))
        .collect()
}

/// One row of the layout: its nodes (in declaration order) and its
/// bounding width/height once `nodesep` is applied.
pub struct LayeredLayout {
    /// Node name -> top-left position, relative to the layout's own local
    /// origin `(0, 0)`.
    pub positions: IndexMap<String, (f64, f64)>,
    pub content_w: f64,
    pub content_h: f64,
}

/// Places every named node into rows by rank (top to bottom, `ranksep`
/// apart) and columns within its row (left to right in declaration order,
/// `nodesep` apart, each row centered under the widest row).
pub fn layered_layout(
    names: &[String],
    edges: &[(String, String)],
    sizes: &IndexMap<String, (f64, f64)>,
    ranksep: f64,
    nodesep: f64,
) -> LayeredLayout {
    let ranks = assign_ranks(names, edges);
    let max_rank = ranks.values().copied().max().unwrap_or(0);

    let mut rows: Vec<Vec<&String>> = vec![Vec::new(); max_rank + 1];
    for n in names {
        rows[ranks[n]].push(n);
    }

    // Pass 1: row widths/heights, laid out from x=0 (re-centered in pass 2).
    struct Row {
        names: Vec<String>,
        xs: Vec<f64>,
        width: f64,
        height: f64,
    }
    let mut computed_rows: Vec<Row> = Vec::new();
    let mut content_w = 0.0_f64;
    for row in &rows {
        let mut x = 0.0_f64;
        let mut height = 0.0_f64;
        let mut xs = Vec::new();
        for n in row {
            let (w, h) = sizes[n.as_str()];
            xs.push(x);
            x += w + nodesep;
            height = height.max(h);
        }
        let width = (x - nodesep).max(0.0);
        content_w = content_w.max(width);
        computed_rows.push(Row {
            names: row.iter().map(|s| (*s).clone()).collect(),
            xs,
            width,
            height,
        });
    }

    let mut positions: IndexMap<String, (f64, f64)> = IndexMap::new();
    let mut y = 0.0_f64;
    for row in &computed_rows {
        if row.names.is_empty() {
            continue;
        }
        let row_offset = (content_w - row.width) / 2.0;
        for (n, x) in row.names.iter().zip(&row.xs) {
            positions.insert(n.clone(), (x + row_offset, y));
        }
        y += row.height + ranksep;
    }
    let content_h = (y - ranksep).max(0.0);

    LayeredLayout {
        positions,
        content_w,
        content_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ns: &[&str]) -> Vec<String> {
        ns.iter().map(|s| s.to_string()).collect()
    }
    fn edges(es: &[(&str, &str)]) -> Vec<(String, String)> {
        es.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn an_edge_naming_an_undeclared_node_is_ignored_rather_than_fatal() {
        // Ply's own fuzz run found this: an edge pointing at a name the
        // caller never declared used to index a map built only from the
        // declared names, and the layout crashed instead of drawing.
        let ranks = assign_ranks(&names(&["a"]), &edges(&[("a", "ghost")]));
        assert_eq!(ranks.len(), 1);
        assert_eq!(ranks["a"], 0);
    }

    #[test]
    fn an_edge_from_an_undeclared_node_does_not_freeze_the_one_it_points_at() {
        // The reverse direction, which the traversal already dropped
        // because it only ever starts from a declared name. Guarded here so
        // the two halves of the rule stay together.
        let ranks = assign_ranks(
            &names(&["a", "b", "c"]),
            &edges(&[("ghost", "b"), ("a", "b"), ("b", "c")]),
        );
        assert_eq!(ranks["a"], 0);
        assert_eq!(ranks["b"], 1);
        assert_eq!(ranks["c"], 2);
    }

    #[test]
    fn a_chain_ranks_in_order() {
        let ranks = assign_ranks(&names(&["a", "b", "c"]), &edges(&[("a", "b"), ("b", "c")]));
        assert_eq!(ranks["a"], 0);
        assert_eq!(ranks["b"], 1);
        assert_eq!(ranks["c"], 2);
    }

    #[test]
    fn isolated_nodes_default_to_rank_zero() {
        let ranks = assign_ranks(&names(&["a", "b"]), &edges(&[]));
        assert_eq!(ranks["a"], 0);
        assert_eq!(ranks["b"], 0);
    }

    #[test]
    fn a_two_cycle_between_ranked_neighbours_still_terminates_and_ranks() {
        // decoder->ring (call) and ring->decoder (flow): a direct 2-cycle.
        // Ranking must not loop forever, and must still separate the pair.
        let ranks = assign_ranks(
            &names(&["feed", "ring", "decoder", "book"]),
            &edges(&[
                ("feed", "ring"),
                ("decoder", "ring"),
                ("ring", "decoder"),
                ("decoder", "book"),
            ]),
        );
        assert_eq!(ranks["feed"], 0);
        assert_eq!(ranks["ring"], 1);
        assert!(ranks["decoder"] > ranks["ring"]);
        assert!(ranks["book"] > ranks["decoder"]);
    }

    #[test]
    fn multiple_sources_share_a_row() {
        let ranks = assign_ranks(
            &names(&["pricing", "db_raw", "migrations", "parser", "risk"]),
            &edges(&[("pricing", "parser"), ("pricing", "risk")]),
        );
        assert_eq!(ranks["pricing"], 0);
        assert_eq!(ranks["db_raw"], 0);
        assert_eq!(ranks["migrations"], 0);
        assert_eq!(ranks["parser"], 1);
        assert_eq!(ranks["risk"], 1);
    }

    #[test]
    fn layered_layout_separates_ranks_by_ranksep_and_columns_by_nodesep() {
        let mut sizes = IndexMap::new();
        sizes.insert("a".to_string(), (100.0, 50.0));
        sizes.insert("b".to_string(), (80.0, 40.0));
        let layout = layered_layout(
            &names(&["a", "b"]),
            &edges(&[("a", "b")]),
            &sizes,
            60.0,
            30.0,
        );
        let (ax, ay) = layout.positions["a"];
        let (bx, by) = layout.positions["b"];
        assert_eq!(ay, 0.0);
        assert_eq!(by, 50.0 + 60.0);
        // single node per row: each row is centered on the wider of the two,
        // so the narrower row ("b", width 80 vs "a"'s 100) is inset by half
        // the difference rather than left-aligned at 0.
        assert_eq!(ax, 0.0);
        assert_eq!(bx, 10.0);
    }

    #[test]
    fn same_rank_nodes_are_placed_nodesep_apart() {
        let mut sizes = IndexMap::new();
        sizes.insert("a".to_string(), (100.0, 50.0));
        sizes.insert("b".to_string(), (80.0, 40.0));
        let layout = layered_layout(&names(&["a", "b"]), &edges(&[]), &sizes, 60.0, 30.0);
        let (ax, _) = layout.positions["a"];
        let (bx, _) = layout.positions["b"];
        assert_eq!(bx - ax, 100.0 + 30.0);
    }
}
