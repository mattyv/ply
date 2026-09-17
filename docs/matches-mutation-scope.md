# Standard matches! mutation scope (PLY-004)

A passing bounded proof may also report W0530 when the static call walk cannot
establish every function reached by the checks. Mutation coverage and proof
coverage describe different runs.

The walker now parses standard bare `matches!` arguments completely and visits
the scrutinee, pattern, and optional guard. This includes helper calls and
function values in contracts, examples, and reached bodies. Alternative patterns
and trailing commas are supported; nested unknown macros still widen the scope.

Potentially shadowing imports (including glob imports and function-local imports)
and macro definitions disable this recognition conservatively across first-party
source. Qualified macros and malformed inputs retain the existing fallback.

The independently written `matchesreach` fixture checks native bounded proof,
fuzzing, examples, and mutation together. Its oracle helper is called only from a
`matches!` guard, and the test checks that cargo-mutants selects that helper and
that W0530 is absent. Unit tests also cover helper-edit invalidation and unsafe
macro fallbacks. cpp-sca receipts remain read-only inputs; no private application
source is added by this fix.
