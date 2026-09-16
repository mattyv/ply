# Nested borrowed byte-slice regression

The cpp-sca reinstall receipts from 2026-09-16 reproduce two defects for
`valid_key_bytes(keys: &[&[u8]]) -> bool`:

- PLY-001: native bounded input generation refused the signature.
- PLY-002: fuzz generation passed owned rows to a borrowed-row API, causing E0308/X0901
  before any case executed.

## Result

The production while-loop body and exact declared postcondition now earn native
`bounded(4)` and execute 256 fuzz cases. Fixed stack backing, independent symbolic row
lengths, and an independent row count cover every list of 0..=4 keys of 0..=4 arbitrary
bytes, including invalid UTF-8. K0510 reports the disjoint backing restriction;
shared-storage aliasing and pointer identity remain outside this domain.
The string adapter and native bounded `&[String]` generation remain outside this fix.

Kani 0.67.0's iterator postcondition produced an isolated failing trace that passed native
playback. An equivalent finite expansion avoids that iterator-model path. Native Rust
comparisons exhaust lists through length three, row lengths through three, and bytes 0/255;
they also cover macro captures, shadowed callback bindings, index types, and the empty domain.
Unknown binding scopes and unsupported iterator forms retain their original code.

Nested byte proofs use `#[kani::proof]`, assume the merged precondition, call the real body,
and explicitly assert the merged postcondition. Safety and unwinding checks remain enabled.
Inline postconditions are expanded only in a private source shadow, so Kani can also check
the called function's inline contract without the original iterator path. Dependency contract
assertions stay enabled. Original application source bytes stay unchanged after verification.

## Acceptance

`cargo test -p ply-e2e --test nestedbytes_fixture --locked -- --nocapture` covers:

- Correct production rule, native bounded(4), and 256 actual fuzz cases.
- Reject-all, accept-all, missed-empty, and missed-nonadjacent-duplicate bodies: each earns
  K0502 and a native test that fails before repair and passes with the correct body.
- Fuzz violation and ordinary Rust replay.
- Independent extra parameters, including names resembling generator helpers.
- Snapshot-helper collisions and mixed borrowed scalar inputs.
- The smallest supported bound, inline/YAML conjunction, precondition handling,
  exact original-source preservation, and a contradictory extra YAML clause.

The original cpp-sca source and issue log are read-only regression inputs.
