//! Scale spike fixture: the ADR-0003 follow-up ("the open risk that gates M3").
//! Throwaway crate -- see tests/spike/scale/SCALE-FINDINGS.md. Kani pinned:
//! cargo-kani 0.67.0 (CBMC 6.8.0), same toolchain as tests/spike/fixture.
//!
//! Every `#[cfg(kani)]` proof module below is named `itemN_*_proofs` to match
//! the numbering in the spike brief and in SCALE-FINDINGS.md's results table.
//! `run.sh` invokes each harness individually (`--exact --harness <path>`) so
//! wall time and verdict are attributable to one experiment at a time.

use std::collections::{BTreeSet, HashMap};
use std::hash::BuildHasherDefault;

// =====================================================================
// Item 1: Vec<u8> as an input, swept over bound N = 1, 2, 4, 8, 16.
//
// `Vec<T>` has NO plain `kani::Arbitrary` impl in 0.67.0 -- only
// `BoundedArbitrary::bounded_any::<N>()` (library/kani/src/bounded_arbitrary.rs)
// or the older `kani::vec::any_vec::<T, N>()` helper it's replacing
// (library/kani/src/vec.rs). Both give a *length-bounded* symbolic Vec: real
// length is itself symbolic, 0..=N. This alone is worth recording: v1 the
// vec-of-supported-types clause needs a length bound to mean anything to Kani.
// =====================================================================

/// Deliberately an iterator-combinator chain (`.iter().map().sum()`), NOT a
/// manual indexed loop -- kept as its own harness group (below) because a
/// first run at N=1 found this chain alone times out CBMC regardless of the
/// (tiny) real length: CBMC unwinds the *generic*
/// `Iterator::fold`/`Map::map_fold`/`Sum::sum` trait machinery, not a loop
/// bounded by the concrete length. See SCALE-FINDINGS.md item 1's note on
/// the iterator-chain confound. `vec_sum_loop` below is the length-scaling
/// experiment this file actually needs; this one measures something else
/// (iterator-combinator cost) and is kept only for that record.
pub fn vec_sum(v: &[u8]) -> u32 {
    v.iter().map(|&x| x as u32).sum()
}

/// The manual-loop counterpart -- same computation, same production idiom as
/// `tools/kernel/src/lib.rs`'s `aggregate_raw` (`for child in &node.children`),
/// which never uses iterator-combinator chains. This is the fair Vec-length
/// scaling test.
pub fn vec_sum_loop(v: &[u8]) -> u32 {
    let mut acc: u32 = 0;
    for i in 0..v.len() {
        acc += v[i] as u32;
    }
    acc
}

/// Isolating variant: loop bound is the compile-time-constant `N` (not the
/// symbolic `v.len()`), guarded per-iteration instead. Tests whether it's
/// specifically the *symbolic loop bound* `0..v.len()` that stalls CBMC, as
/// opposed to indexing a Vec at all.
pub fn vec_sum_loop_const_bound_n1(v: &[u8]) -> u32 {
    let mut acc: u32 = 0;
    for i in 0..1usize {
        if i < v.len() {
            acc += v[i] as u32;
        }
    }
    acc
}

#[cfg_attr(kani, kani::ensures(|result| *result <= 255 * v.len() as u32))]
pub fn vec_sum_loop_contract(v: &Vec<u8>) -> u32 {
    let mut acc: u32 = 0;
    for i in 0..v.len() {
        acc += v[i] as u32;
    }
    acc
}

#[cfg(kani)]
mod item1_vec_proofs {
    use super::*;

    macro_rules! vec_harness {
        ($fn_name:ident, $n:expr) => {
            #[kani::proof]
            fn $fn_name() {
                let v = kani::vec::any_vec::<u8, $n>();
                vec_sum(&v);
            }
        };
    }
    vec_harness!(check_vec_sum_n1, 1);
    vec_harness!(check_vec_sum_n2, 2);
    vec_harness!(check_vec_sum_n4, 4);
    vec_harness!(check_vec_sum_n8, 8);
    vec_harness!(check_vec_sum_n16, 16);

    // NOTE: no iterator-chain *contract* harness group -- superseded by the
    // loop-based contract harnesses below before it was ever run cleanly
    // (the plain, non-contract iterator-chain group above already
    // demonstrated the confound at n=1,2,4; see SCALE-FINDINGS.md item 1).

    macro_rules! vec_loop_harness {
        ($fn_name:ident, $n:expr) => {
            #[kani::proof]
            fn $fn_name() {
                let v = kani::vec::any_vec::<u8, $n>();
                vec_sum_loop(&v);
            }
        };
    }
    vec_loop_harness!(check_vec_sum_loop_n1, 1);
    vec_loop_harness!(check_vec_sum_loop_n2, 2);
    vec_loop_harness!(check_vec_sum_loop_n4, 4);
    vec_loop_harness!(check_vec_sum_loop_n8, 8);
    vec_loop_harness!(check_vec_sum_loop_n16, 16);

    // Item 7: does an explicit `#[kani::unwind(N)]` rescue the `0..v.len()`
    // symbolic-bound loop the same way it rescued item 6's recursion? N=2 is
    // one more than the real max length (1), matching the same "K+1" margin
    // used for check_count_nodes_fixed_depth1_unwind3.
    #[kani::proof]
    #[kani::unwind(2)]
    fn check_vec_sum_loop_n1_unwind2() {
        let v = kani::vec::any_vec::<u8, 1>();
        vec_sum_loop(&v);
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn check_vec_sum_loop_n4_unwind5() {
        let v = kani::vec::any_vec::<u8, 4>();
        vec_sum_loop(&v);
    }

    #[kani::proof]
    #[kani::unwind(9)]
    fn check_vec_sum_loop_n8_unwind9() {
        let v = kani::vec::any_vec::<u8, 8>();
        vec_sum_loop(&v);
    }

    #[kani::proof]
    #[kani::unwind(17)]
    fn check_vec_sum_loop_n16_unwind17() {
        let v = kani::vec::any_vec::<u8, 16>();
        vec_sum_loop(&v);
    }

    macro_rules! vec_loop_contract_harness {
        ($fn_name:ident, $n:expr) => {
            #[kani::proof_for_contract(vec_sum_loop_contract)]
            fn $fn_name() {
                let v = kani::vec::any_vec::<u8, $n>();
                vec_sum_loop_contract(&v);
            }
        };
    }
    vec_loop_contract_harness!(check_vec_sum_loop_contract_n1, 1);
    vec_loop_contract_harness!(check_vec_sum_loop_contract_n2, 2);
    vec_loop_contract_harness!(check_vec_sum_loop_contract_n4, 4);
    vec_loop_contract_harness!(check_vec_sum_loop_contract_n8, 8);
    vec_loop_contract_harness!(check_vec_sum_loop_contract_n16, 16);

    // Isolating experiment: does merely CONSTRUCTING a bounded Vec (no loop,
    // no downstream use at all) cost anything, at N=0 -- the smallest
    // possible bound? If this alone times out, the cost is in
    // `any_vec`/`BoundedArbitrary`'s own construction machinery
    // (`Vec::from([T; N])` + `truncate` + `shrink_to_fit`, all with a
    // symbolic real-length), not in any loop or downstream computation.
    #[kani::proof]
    fn check_vec_construct_only_n0() {
        let _v = kani::vec::any_vec::<u8, 0>();
    }

    #[kani::proof]
    fn check_vec_construct_only_n1() {
        let _v = kani::vec::any_vec::<u8, 1>();
    }

    #[kani::proof]
    fn check_vec_sum_loop_const_bound_n1() {
        let v = kani::vec::any_vec::<u8, 1>();
        vec_sum_loop_const_bound_n1(&v);
    }
}

// =====================================================================
// Item 2: fixed-size array [u8; N] for the same N values -- the spec's
// suggested alternative shape. Concrete (non-generic) functions per N,
// deliberately: The-Ply-Spec.md §5.4b already says a generic fn is checkable
// only through one concrete `check_with` instantiation, so this mirrors what
// Ply would actually generate rather than testing Kani's generic-contract
// support (a separate, unrelated question).
// =====================================================================

// Manual indexed loop, not `.iter().map().sum()` -- same reason as
// `vec_sum_loop` above: the iterator-combinator chain measures CBMC's
// handling of generic trait machinery, not container-length scaling.
macro_rules! array_item {
    ($sum_fn:ident, $contract_fn:ident, $n:expr) => {
        pub fn $sum_fn(a: [u8; $n]) -> u32 {
            let mut acc: u32 = 0;
            for i in 0..$n {
                acc += a[i] as u32;
            }
            acc
        }

        #[cfg_attr(kani, kani::ensures(|result| *result <= 255 * $n))]
        pub fn $contract_fn(a: [u8; $n]) -> u32 {
            let mut acc: u32 = 0;
            for i in 0..$n {
                acc += a[i] as u32;
            }
            acc
        }
    };
}

array_item!(array_sum_n1, array_sum_contract_n1, 1);
array_item!(array_sum_n2, array_sum_contract_n2, 2);
array_item!(array_sum_n4, array_sum_contract_n4, 4);
array_item!(array_sum_n8, array_sum_contract_n8, 8);
array_item!(array_sum_n16, array_sum_contract_n16, 16);

#[cfg(kani)]
mod item2_array_proofs {
    use super::*;

    macro_rules! array_harness {
        ($fn_name:ident, $target:ident) => {
            #[kani::proof]
            fn $fn_name() {
                let a = kani::any();
                $target(a);
            }
        };
    }
    array_harness!(check_array_sum_n1, array_sum_n1);
    array_harness!(check_array_sum_n2, array_sum_n2);
    array_harness!(check_array_sum_n4, array_sum_n4);
    array_harness!(check_array_sum_n8, array_sum_n8);
    array_harness!(check_array_sum_n16, array_sum_n16);

    macro_rules! array_contract_harness {
        ($fn_name:ident, $target:ident) => {
            #[kani::proof_for_contract($target)]
            fn $fn_name() {
                let a = kani::any();
                $target(a);
            }
        };
    }
    array_contract_harness!(check_array_sum_contract_n1, array_sum_contract_n1);
    array_contract_harness!(check_array_sum_contract_n2, array_sum_contract_n2);
    array_contract_harness!(check_array_sum_contract_n4, array_sum_contract_n4);
    array_contract_harness!(check_array_sum_contract_n8, array_sum_contract_n8);
    array_contract_harness!(check_array_sum_contract_n16, array_sum_contract_n16);
}

// =====================================================================
// Item 3: Option<T> / Result<T, E> of a scalar.
// =====================================================================

pub fn option_default(x: Option<u32>) -> u32 {
    match x {
        Some(v) => v,
        None => 0,
    }
}

pub fn result_default(x: Result<u32, u8>) -> u32 {
    match x {
        Ok(v) => v,
        Err(_) => 0,
    }
}

#[cfg(kani)]
mod item3_option_result_proofs {
    use super::*;

    #[kani::proof]
    fn check_option_default() {
        let x: Option<u32> = kani::any();
        option_default(x);
    }

    #[kani::proof]
    fn check_result_default() {
        let x: Result<u32, u8> = kani::any();
        result_default(x);
    }
}

// =====================================================================
// Item 4: a struct containing a Vec vs the same struct with a fixed array.
//
// `#[derive(kani::BoundedArbitrary)]` + `#[bounded]` on the Vec field is the
// documented mechanism (library/kani_macros/src/derive_bounded.rs) -- plain
// `#[derive(kani::Arbitrary)]` does not compile for a struct with a `Vec`
// field at all, because `Vec<T>` has no plain `Arbitrary` impl (see item 1).
// =====================================================================

#[cfg_attr(kani, derive(kani::BoundedArbitrary))]
pub struct WithVec {
    #[cfg_attr(kani, bounded)]
    pub data: Vec<u8>,
    pub tag: u8,
}

#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct WithArray4 {
    pub data: [u8; 4],
    pub tag: u8,
}

#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct WithArray16 {
    pub data: [u8; 16],
    pub tag: u8,
}

// Manual indexed loops throughout -- see item 1's note on the
// iterator-combinator confound found mid-sweep.
pub fn with_vec_sum(s: &WithVec) -> u32 {
    let mut acc: u32 = 0;
    for i in 0..s.data.len() {
        acc += s.data[i] as u32;
    }
    acc + s.tag as u32
}

pub fn with_array4_sum(s: &WithArray4) -> u32 {
    let mut acc: u32 = 0;
    for i in 0..4 {
        acc += s.data[i] as u32;
    }
    acc + s.tag as u32
}

pub fn with_array16_sum(s: &WithArray16) -> u32 {
    let mut acc: u32 = 0;
    for i in 0..16 {
        acc += s.data[i] as u32;
    }
    acc + s.tag as u32
}

#[cfg(kani)]
mod item4_struct_proofs {
    use super::*;

    macro_rules! bounded_struct_harness {
        ($fn_name:ident, $n:expr) => {
            #[kani::proof]
            fn $fn_name() {
                let s: WithVec = kani::bounded_any::<WithVec, $n>();
                with_vec_sum(&s);
            }
        };
    }
    bounded_struct_harness!(check_with_vec_n4, 4);
    bounded_struct_harness!(check_with_vec_n16, 16);

    #[kani::proof]
    fn check_with_array4() {
        let s: WithArray4 = kani::any();
        with_array4_sum(&s);
    }

    #[kani::proof]
    fn check_with_array16() {
        let s: WithArray16 = kani::any();
        with_array16_sum(&s);
    }
}

// =====================================================================
// Item 5: BTreeSet<u8> / HashMap<u8, u8>, flat (non-recursive) inputs,
// with element-count bound N = 0, 1, 2. Isolates "does a bare collection
// input cost anything at trivially small size" from item 6's recursive
// clone/extend pattern, which is where ADR-0003 says the kernel actually
// stalled.
//
// `BTreeSet`/`BTreeMap` have NO `BoundedArbitrary` impl anywhere in the
// pinned kani library (checked: library/kani/src/bounded_arbitrary.rs
// implements it only for Box<[T]>, Vec<T>, String, HashMap, HashSet) --
// built via a bounded Vec collected into a BTreeSet by hand, which is itself
// a finding: §5.4b cannot claim BTreeSet/BTreeMap support "for free".
//
// `HashMap<u8,u8>`'s *default* hasher is `RandomState`, which has no
// `Arbitrary`/`BoundedArbitrary` impl at all (it's seeded from OS randomness,
// not deterministically constructible) -- so the default-hasher case isn't
// merely slow, it doesn't compile. `HashMap<u8,u8,BuildHasherDefault<DefaultHasher>>`
// is what the kani library actually ships `BoundedArbitrary` for.
// =====================================================================

// Plain `for` loops (not `.iter().map().sum()` chains) -- neither collection
// is index-addressable, so this is the closest analogue to item 1's
// `vec_sum_loop`, and matches the kernel's own idiom (`StatusSet::extend`'s
// `for status in iter`).
pub fn btreeset_sum(s: &BTreeSet<u8>) -> u32 {
    let mut acc: u32 = 0;
    for &x in s {
        acc += x as u32;
    }
    acc
}

type DeterministicHashMap = HashMap<u8, u8, BuildHasherDefault<std::collections::hash_map::DefaultHasher>>;

pub fn hashmap_sum(m: &DeterministicHashMap) -> u32 {
    let mut acc: u32 = 0;
    for (_, &v) in m {
        acc += v as u32;
    }
    acc
}

#[cfg(kani)]
mod item5_collections_proofs {
    use super::*;

    macro_rules! btreeset_harness {
        ($fn_name:ident, $n:expr) => {
            #[kani::proof]
            fn $fn_name() {
                let v = kani::vec::any_vec::<u8, $n>();
                let s: BTreeSet<u8> = v.into_iter().collect();
                btreeset_sum(&s);
            }
        };
    }
    btreeset_harness!(check_btreeset_n0, 0);
    btreeset_harness!(check_btreeset_n1, 1);
    btreeset_harness!(check_btreeset_n2, 2);

    // Isolating experiment: build the BTreeSet the same way Kani's own
    // `BoundedArbitrary` impl for HashMap/HashSet does (bounded_arbitrary.rs)
    // -- a CONSTANT-bound `for _ in 0..N` loop with a per-iteration coin
    // flip, insert-by-insert -- instead of `any_vec().collect()`, whose
    // `any_vec` internally loops a SYMBOLIC real-length. If this is fast
    // where `check_btreeset_n0` timed out, the deciding factor is "constant
    // vs symbolic loop bound", not "BTreeSet vs HashMap".
    macro_rules! btreeset_const_bound_harness {
        ($fn_name:ident, $n:expr) => {
            #[kani::proof]
            fn $fn_name() {
                let mut s: BTreeSet<u8> = BTreeSet::new();
                for _ in 0..$n {
                    if kani::any() {
                        s.insert(kani::any());
                    }
                }
                btreeset_sum(&s);
            }
        };
    }
    btreeset_const_bound_harness!(check_btreeset_const_bound_n0, 0);
    btreeset_const_bound_harness!(check_btreeset_const_bound_n2, 2);

    macro_rules! hashmap_harness {
        ($fn_name:ident, $n:expr) => {
            #[kani::proof]
            fn $fn_name() {
                let m: DeterministicHashMap = kani::bounded_any::<DeterministicHashMap, $n>();
                hashmap_sum(&m);
            }
        };
    }
    hashmap_harness!(check_hashmap_deterministic_n0, 0);
    hashmap_harness!(check_hashmap_deterministic_n1, 1);
    hashmap_harness!(check_hashmap_deterministic_n2, 2);

    // NOTE: there is deliberately no `check_hashmap_default_hasher_*` harness
    // here. `HashMap<u8,u8>` (RandomState) does not implement `Arbitrary` or
    // `BoundedArbitrary` -- attempting `kani::any::<HashMap<u8,u8>>()` is a
    // compile error, not a slow/timing-out proof. See SCALE-FINDINGS.md item 5.
}

// =====================================================================
// Item 6: a recursive structure -- this IS the kernel's actual shape
// (tools/kernel/src/lib.rs: `VerdictNode { .. children: Vec<VerdictNode> }`,
// and the doc comment on `StatusSet` naming `aggregate_raw`'s
// clone-then-extend-per-recursive-call pattern as exactly what stalled CBMC
// on a `BTreeSet`-shaped field).
//
// Symbolic tree input is a hand-rolled, depth-bounded shim -- bool flags
// choosing whether each child slot is present, mirroring
// tools/kernel/src/lib.rs's own `kani_proofs::SymTree`/`SymLeaf` shim, at
// three depths (max width 2 per level, matching the kernel's shim exactly):
//   depth1 = root + <=2 leaf children            (<=3 nodes; this is
//                                                  the kernel's own SymTree
//                                                  shape, already reported
//                                                  as unprovable)
//   depth2 = root + <=2 children, each with <=2 children of its own (<=7 nodes)
//   depth3 = one more level down (<=15 nodes)
//
// Two computations run over each depth: `count_nodes` (trivial recursive
// walk, no per-call collection algorithm -- the "is it just depth" control)
// and `collect_tags_*` (clone + extend a set on every recursive call, the
// actual `aggregate_raw` pattern) in both the `BTreeSet<u8>` and `u8`-bitmask
// shapes, so the fix the kernel actually shipped is directly re-tested here
// on the same toolchain.
// =====================================================================

#[derive(Clone)]
pub struct TreeNode {
    pub tags: BTreeSet<u8>,
    pub children: Vec<TreeNode>,
}

// A recursive struct can't hold `Vec<Self>` and stay `Copy`; only the `tags`
// field is `Copy`-shaped (`u8`, matching the kernel's own `StatusSet`), same
// as `StatusSet` (Copy) living inside the non-Copy `VerdictNode`.
#[derive(Clone)]
pub struct BitmaskNode {
    pub tags: u8,
    pub children: Vec<BitmaskNode>,
}

// Manual `for` loop, not `.iter().map().sum()` -- same iterator-combinator
// confound as item 1's `vec_sum`. This is meant as the "is it just depth"
// control against `collect_tags_btreeset` below, so the only difference
// between the two must be "trivial add" vs "BTreeSet clone+extend", not
// also "combinator chain vs manual loop".
pub fn count_nodes(n: &TreeNode) -> u32 {
    let mut total: u32 = 1;
    for child in &n.children {
        total += count_nodes(child);
    }
    total
}

/// Isolating experiment: same shape (a node with up to 2 children, tag-only
/// payload) but children live in a compile-time-constant-size
/// `[Option<Box<Self>>; 2]`, not a `Vec<Self>` -- so `for slot in
/// &n.children` has a CONSTANT loop bound, matching item 1's
/// `vec_sum_loop_const_bound_n1` finding. If this verifies where
/// `check_count_nodes_depth1`/`check_collect_bitmask_depth1` (Vec-shaped
/// children) time out, the bottleneck is specifically "recursing over a Vec
/// of symbolic length", not depth or recursion itself.
pub struct FixedNode {
    pub tag: u8,
    pub children: [Option<Box<FixedNode>>; 2],
}

pub fn count_nodes_fixed(n: &FixedNode) -> u32 {
    let mut total: u32 = 1;
    for slot in &n.children {
        if let Some(child) = slot {
            total += count_nodes_fixed(child);
        }
    }
    total
}

/// Mirrors `aggregate_raw` (tools/kernel/src/lib.rs) exactly: clone this
/// node's own set, then `extend` it with every child's recursively-collected
/// set, once per recursive call -- the pattern the kernel's doc comment
/// names as what CBMC unwinds without bound.
pub fn collect_tags_btreeset(n: &TreeNode) -> BTreeSet<u8> {
    let mut acc = n.tags.clone();
    for child in &n.children {
        acc.extend(collect_tags_btreeset(child));
    }
    acc
}

/// Same computation, `StatusSet`-shaped: a `u8` bitmask, `Copy`, unioned by
/// `|=` -- the kernel's actual fix, re-measured here.
pub fn collect_tags_bitmask(n: &BitmaskNode) -> u8 {
    let mut acc = n.tags;
    for child in &n.children {
        acc |= collect_tags_bitmask(child);
    }
    acc
}

#[cfg(kani)]
mod item6_recursive_proofs {
    use super::*;

    fn sym_tags3() -> (bool, bool, bool) {
        (kani::any(), kani::any(), kani::any())
    }

    fn tags_set((a, b, c): (bool, bool, bool)) -> BTreeSet<u8> {
        let mut s = BTreeSet::new();
        if a {
            s.insert(0);
        }
        if b {
            s.insert(1);
        }
        if c {
            s.insert(2);
        }
        s
    }

    fn tags_mask((a, b, c): (bool, bool, bool)) -> u8 {
        (a as u8) | ((b as u8) << 1) | ((c as u8) << 2)
    }

    fn leaf_set() -> TreeNode {
        TreeNode { tags: tags_set(sym_tags3()), children: Vec::new() }
    }
    fn leaf_mask() -> BitmaskNode {
        BitmaskNode { tags: tags_mask(sym_tags3()), children: Vec::new() }
    }

    fn maybe_push_set(children: &mut Vec<TreeNode>, node: TreeNode) {
        if kani::any() {
            children.push(node);
        }
    }
    fn maybe_push_mask(children: &mut Vec<BitmaskNode>, node: BitmaskNode) {
        if kani::any() {
            children.push(node);
        }
    }

    fn node_set(children: Vec<TreeNode>) -> TreeNode {
        TreeNode { tags: tags_set(sym_tags3()), children }
    }
    fn node_mask(children: Vec<BitmaskNode>) -> BitmaskNode {
        BitmaskNode { tags: tags_mask(sym_tags3()), children }
    }

    fn build_depth1_set() -> TreeNode {
        let mut children = Vec::new();
        maybe_push_set(&mut children, leaf_set());
        maybe_push_set(&mut children, leaf_set());
        node_set(children)
    }
    fn build_depth2_set() -> TreeNode {
        let mut children = Vec::new();
        maybe_push_set(&mut children, build_depth1_set());
        maybe_push_set(&mut children, build_depth1_set());
        node_set(children)
    }
    fn build_depth3_set() -> TreeNode {
        let mut children = Vec::new();
        maybe_push_set(&mut children, build_depth2_set());
        maybe_push_set(&mut children, build_depth2_set());
        node_set(children)
    }

    fn build_depth1_mask() -> BitmaskNode {
        let mut children = Vec::new();
        maybe_push_mask(&mut children, leaf_mask());
        maybe_push_mask(&mut children, leaf_mask());
        node_mask(children)
    }
    fn build_depth2_mask() -> BitmaskNode {
        let mut children = Vec::new();
        maybe_push_mask(&mut children, build_depth1_mask());
        maybe_push_mask(&mut children, build_depth1_mask());
        node_mask(children)
    }
    fn build_depth3_mask() -> BitmaskNode {
        let mut children = Vec::new();
        maybe_push_mask(&mut children, build_depth2_mask());
        maybe_push_mask(&mut children, build_depth2_mask());
        node_mask(children)
    }

    fn build_fixed_leaf() -> FixedNode {
        FixedNode { tag: kani::any(), children: [None, None] }
    }

    fn build_fixed_depth1() -> FixedNode {
        let slot_a = if kani::any() { Some(Box::new(build_fixed_leaf())) } else { None };
        let slot_b = if kani::any() { Some(Box::new(build_fixed_leaf())) } else { None };
        FixedNode { tag: kani::any(), children: [slot_a, slot_b] }
    }

    // Depth-0 control: a `FixedNode` whose recursive type is present but
    // never actually recurses (children forced to [None, None]). Isolates
    // "the type is self-referential" from "self-reference actually resolves
    // at least once".
    #[kani::proof]
    fn check_count_nodes_fixed_depth0() {
        count_nodes_fixed(&build_fixed_leaf());
    }

    #[kani::proof]
    fn check_count_nodes_fixed_depth1() {
        count_nodes_fixed(&build_fixed_depth1());
    }

    // Item 7: does an explicit recursion-unwind bound fix it? CBMC's own
    // diagnostic said "Unwinding recursion count_nodes_fixed iteration
    // 307/308" with NO bound set -- it treats the recursive call itself as
    // an unbounded unwind target, unrelated to the data's real depth (1).
    // `build_fixed_depth1` recurses at most 2 calls deep (root -> child ->
    // count_nodes_fixed on a childless leaf), so an unwind bound of 3 should
    // be more than enough if this is really the fix.
    #[kani::proof]
    #[kani::unwind(3)]
    fn check_count_nodes_fixed_depth1_unwind3() {
        count_nodes_fixed(&build_fixed_depth1());
    }

    #[kani::proof]
    fn check_count_nodes_depth1() {
        count_nodes(&build_depth1_set());
    }
    #[kani::proof]
    fn check_count_nodes_depth2() {
        count_nodes(&build_depth2_set());
    }
    #[kani::proof]
    fn check_count_nodes_depth3() {
        count_nodes(&build_depth3_set());
    }

    #[kani::proof]
    fn check_collect_btreeset_depth1() {
        collect_tags_btreeset(&build_depth1_set());
    }
    #[kani::proof]
    fn check_collect_btreeset_depth2() {
        collect_tags_btreeset(&build_depth2_set());
    }
    #[kani::proof]
    fn check_collect_btreeset_depth3() {
        collect_tags_btreeset(&build_depth3_set());
    }

    #[kani::proof]
    fn check_collect_bitmask_depth1() {
        collect_tags_bitmask(&build_depth1_mask());
    }

    // Item 7, the decisive combined test: does an explicit unwind bound
    // rescue the KERNEL'S ACTUAL SHAPE -- recursion THROUGH a Vec<Self>
    // field of symbolic length (not the isolated FixedNode/array case above)?
    // `#[kani::unwind(N)]` sets one bound applied to every loop/recursion
    // site in the harness, so this must cover both the >=2-deep recursion
    // AND the <=2-iteration `for child in &n.children` loop at once.
    #[kani::proof]
    #[kani::unwind(4)]
    fn check_collect_bitmask_depth1_unwind4() {
        collect_tags_bitmask(&build_depth1_mask());
    }
    #[kani::proof]
    fn check_collect_bitmask_depth2() {
        collect_tags_bitmask(&build_depth2_mask());
    }
    #[kani::proof]
    fn check_collect_bitmask_depth3() {
        collect_tags_bitmask(&build_depth3_mask());
    }

    // Item 7 variant: same depth1 BTreeSet case, with an explicit
    // `#[kani::unwind(n)]` bound instead of the default/CLI-driven one, to
    // see whether a code-level unwind annotation changes the outcome. CLI
    // flags (`--unwind`, `--solver`, `--cbmc-args -- --object-bits`) are
    // exercised from run.sh against `check_collect_btreeset_depth1` directly
    // -- no source change needed for those.
    #[kani::proof]
    #[kani::unwind(4)]
    fn check_collect_btreeset_depth1_unwind4() {
        collect_tags_btreeset(&build_depth1_set());
    }
}
