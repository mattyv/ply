#!/usr/bin/env bash
# Driver used while gathering SCALE-FINDINGS.md data. Takes harness paths on
# argv, runs each individually with a capped timeout, appends one row per
# harness to results.csv (name,wall_seconds,verdict,note). Not the polished
# deliverable script (that's run.sh) -- this is the raw sweep runner.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/fixture"

TIMEOUT="${SWEEP_TIMEOUT:-60s}"
FLAGS=(-Z function-contracts -Z stubbing -Z unstable-options --harness-timeout "$TIMEOUT")
OUT="${SWEEP_OUT:-../results.csv}"

for harness in "$@"; do
  start=$(date +%s)
  log=$(cargo kani "${FLAGS[@]}" --exact --harness "$harness" 2>&1)
  status=$?
  end=$(date +%s)
  elapsed=$((end - start))

  # Order matters: Kani prints "VERIFICATION:- FAILED" for a CBMC timeout
  # too (with "CBMC timed out" as the reason), so the timeout check must
  # run BEFORE the generic FAILED check or a timeout gets misfiled as a
  # genuine contract/assertion failure.
  if echo "$log" | grep -qi "CBMC timed out\|harness timed out\|timed out"; then
    verdict="TIMEOUT@${TIMEOUT}"
  elif echo "$log" | grep -q "VERIFICATION:- SUCCESSFUL"; then
    verdict="VERIFIED"
  elif echo "$log" | grep -q "VERIFICATION:- FAILED"; then
    verdict="FAILED(assertion)"
  elif [ $status -ne 0 ]; then
    verdict="ERROR(exit=$status)"
  else
    verdict="UNKNOWN"
  fi

  failcount=$(echo "$log" | grep -oE '[0-9]+ of [0-9]+ failed' | tail -1)
  echo "${harness},${elapsed}s,${verdict},\"${failcount}\"" | tee -a "$OUT"
done
