#!/usr/bin/env bash
# Scale spike: re-run every item in tests/spike/scale/SCALE-FINDINGS.md end to
# end. This is the follow-up to tests/spike/FINDINGS.md (M0) -- ADR-0003's
# "open risk that gates M3": what collection-shaped code can Kani 0.67.0
# actually verify?
#
# Pinned toolchain: cargo-kani 0.67.0 / CBMC 6.8.0, same as tests/spike/run.sh.
# Fixture: tests/spike/scale/fixture, its own `[workspace]` root (not a member
# of tools/Cargo.toml or tests/spike/fixture's workspace).
#
# Every individual `cargo kani` call is capped with --harness-timeout so
# nothing can run away (a harness ran over an hour with no verdict once,
# before this rule existed). Default cap is 60s; two calls below
# deliberately override it (--solver kissat at 60s, and the hardest
# combined-recursion case at 180s) to answer specific questions in
# SCALE-FINDINGS.md item 7 -- both are still capped, never open-ended.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

echo "== cargo kani --version =="
cargo kani --version

echo
echo "== Compile check (fast: --only-codegen, no CBMC) =="
(cd fixture && cargo kani -Z function-contracts -Z stubbing -Z unstable-options --only-codegen)

rm -f results.csv
echo "harness,wall,verdict,failcount" > results.csv

echo
echo "== Item 1: Vec<u8>, N = 1,2,4,8,16 =="
echo "-- iterator-chain variant (.iter().map().sum()) -- KNOWN CONFOUND, kept for the record --"
SWEEP_TIMEOUT=120s ./sweep.sh \
  item1_vec_proofs::check_vec_sum_n1 \
  item1_vec_proofs::check_vec_sum_n2 \
  item1_vec_proofs::check_vec_sum_n4 \
  item1_vec_proofs::check_vec_sum_n8 \
  item1_vec_proofs::check_vec_sum_n16

echo "-- manual-loop variant (the fair length-scaling test), no explicit unwind --"
./sweep.sh \
  item1_vec_proofs::check_vec_sum_loop_n1 \
  item1_vec_proofs::check_vec_sum_loop_n2 \
  item1_vec_proofs::check_vec_sum_loop_n4 \
  item1_vec_proofs::check_vec_sum_loop_n8 \
  item1_vec_proofs::check_vec_sum_loop_n16 \
  item1_vec_proofs::check_vec_sum_loop_contract_n1

echo "-- construction only, no loop at all --"
./sweep.sh \
  item1_vec_proofs::check_vec_construct_only_n0 \
  item1_vec_proofs::check_vec_construct_only_n1

echo "-- constant-bound loop (0..1 + guard, not 0..v.len()) --"
./sweep.sh item1_vec_proofs::check_vec_sum_loop_const_bound_n1

echo "-- WITH an explicit #[kani::unwind(N+1)] -- the fix --"
./sweep.sh \
  item1_vec_proofs::check_vec_sum_loop_n1_unwind2 \
  item1_vec_proofs::check_vec_sum_loop_n4_unwind5 \
  item1_vec_proofs::check_vec_sum_loop_n8_unwind9 \
  item1_vec_proofs::check_vec_sum_loop_n16_unwind17

echo
echo "== Item 2: [u8; N], N = 1,2,4,8,16 (manual loop, constant bound) =="
./sweep.sh \
  item2_array_proofs::check_array_sum_n1 \
  item2_array_proofs::check_array_sum_n2 \
  item2_array_proofs::check_array_sum_n4 \
  item2_array_proofs::check_array_sum_n8 \
  item2_array_proofs::check_array_sum_n16 \
  item2_array_proofs::check_array_sum_contract_n4

echo
echo "== Item 3: Option<u32> / Result<u32,u8> =="
./sweep.sh \
  item3_option_result_proofs::check_option_default \
  item3_option_result_proofs::check_result_default

echo
echo "== Item 4: struct { Vec<u8> } vs struct { [u8; N] } =="
./sweep.sh \
  item4_struct_proofs::check_with_array4 \
  item4_struct_proofs::check_with_array16 \
  item4_struct_proofs::check_with_vec_n4

echo
echo "== Item 5: BTreeSet<u8> / HashMap<u8,u8>, N = 0,1,2 =="
echo "-- BTreeSet via any_vec().collect() (symbolic-length construction) --"
./sweep.sh \
  item5_collections_proofs::check_btreeset_n0 \
  item5_collections_proofs::check_btreeset_n1 \
  item5_collections_proofs::check_btreeset_n2

echo "-- BTreeSet via a constant-bound insert loop (Kani's own HashMap idiom) --"
./sweep.sh \
  item5_collections_proofs::check_btreeset_const_bound_n0 \
  item5_collections_proofs::check_btreeset_const_bound_n2

echo "-- HashMap<u8,u8,BuildHasherDefault<DefaultHasher>> (Kani's built-in BoundedArbitrary) --"
./sweep.sh \
  item5_collections_proofs::check_hashmap_deterministic_n0 \
  item5_collections_proofs::check_hashmap_deterministic_n1 \
  item5_collections_proofs::check_hashmap_deterministic_n2
echo "   (HashMap<u8,u8> with the DEFAULT hasher (RandomState) is not exercised:"
echo "   it has no Arbitrary/BoundedArbitrary impl at all -- compile error, not a timeout.)"

echo
echo "== Item 6: recursive tree (the kernel's actual shape), depth 1..3 =="
echo "-- baseline: count only, BTreeSet-tagged nodes (Vec<Self> children) --"
./sweep.sh \
  item6_recursive_proofs::check_count_nodes_depth1 \
  item6_recursive_proofs::check_count_nodes_depth2 \
  item6_recursive_proofs::check_count_nodes_depth3

echo "-- the kernel's real pattern: clone+extend a BTreeSet per recursive call --"
./sweep.sh \
  item6_recursive_proofs::check_collect_btreeset_depth1 \
  item6_recursive_proofs::check_collect_btreeset_depth2 \
  item6_recursive_proofs::check_collect_btreeset_depth3

echo "-- the kernel's actual fix: a u8 bitmask instead of BTreeSet --"
./sweep.sh \
  item6_recursive_proofs::check_collect_bitmask_depth1 \
  item6_recursive_proofs::check_collect_bitmask_depth2 \
  item6_recursive_proofs::check_collect_bitmask_depth3

echo "-- isolating: fixed-size [Option<Box<Self>>; 2] children instead of Vec<Self> --"
./sweep.sh \
  item6_recursive_proofs::check_count_nodes_fixed_depth0 \
  item6_recursive_proofs::check_count_nodes_fixed_depth1

echo "-- item 7: does an explicit unwind bound rescue recursion alone? --"
./sweep.sh item6_recursive_proofs::check_count_nodes_fixed_depth1_unwind3

echo "-- item 7: does it rescue the COMBINED recursion + Vec<Self> children case? --"
./sweep.sh item6_recursive_proofs::check_collect_bitmask_depth1_unwind4

echo
echo "== Item 7: flag variance on the cleanest failing case =="
echo "-- --solver kissat (default is CaDiCaL) --"
(cd fixture && cargo kani -Z function-contracts -Z stubbing -Z unstable-options \
  --harness-timeout 60s --solver kissat --exact \
  --harness item6_recursive_proofs::check_count_nodes_fixed_depth1 || true)

echo "-- the hardest combined case again, at a much longer cap (180s), to see"
echo "   whether it is 'just slow' or genuinely stuck (it is genuinely large:"
echo "   symbolic execution alone took ~104s and produced 64,147 VCCs) --"
(cd fixture && cargo kani -Z function-contracts -Z stubbing -Z unstable-options \
  --harness-timeout 180s --exact \
  --harness item6_recursive_proofs::check_collect_bitmask_depth1_unwind4 || true)

echo
echo "Done. See tests/spike/scale/SCALE-FINDINGS.md for the results table and"
echo "conclusions, tests/spike/scale/results.csv for this run's raw rows."
