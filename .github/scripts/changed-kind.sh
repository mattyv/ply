#!/usr/bin/env bash
# Says whether a range of commits touches code, or only documentation.
#
# Prints exactly one line: `code` or `docs`. The workflow reads it to decide
# whether the slow jobs -- the end-to-end suite that installs Kani, and the
# kernel mutation run -- have anything to look at. The fast `product` job runs
# regardless: that is where documentation *is* checked (the spec-consistency
# test, the drawing drift check), so a docs change still gets the checks that
# can see it.
#
# "Documentation" is a closed list, and anything not on it is code. Erring
# that way is the whole design: a file this script does not recognise runs
# the full suite, so a new kind of source file added later can never be
# silently waved through as prose. The list:
#
#   *.md everywhere            prose, including the spec and this repo's TODO
#   docs/**                    the documentation tree (prose, drawings, text
#                              forms, the architecture page)
#   .archi/**                  the architecture diagrams bundle
#   vetting/*.svg, *.txt       generated drawings and text forms beside the
#   demos/*.svg, *.txt         scenarios; pinned byte for byte by the drift
#                              check in the fast job, so a change here is
#                              still checked, just not by Kani
#
# Not on the list, deliberately: any `ply.yaml` (the end-to-end suite reads
# fixture documents), anything under `demos/verified-green` (a real crate),
# and this workflow itself.
#
# Usage: changed-kind.sh <base> <head>
# Lives in its own file rather than inline in the workflow so the same code
# CI runs can be run by hand against a real range: see the test beside it.
set -euo pipefail

base="${1:?base ref}"
head="${2:?head ref}"

changed=$(git diff --name-only "$base" "$head")

# An empty diff is not "documentation only"; it is "nothing to classify",
# and the safe answer to that is the full suite.
if [ -z "$changed" ]; then
  echo code
  exit 0
fi

docs_only=1
while IFS= read -r path; do
  case "$path" in
    *.md) ;;
    docs/*) ;;
    .archi/*) ;;
    vetting/*.svg|vetting/*.txt) ;;
    demos/*.svg|demos/*.txt) ;;
    *) docs_only=0; break ;;
  esac
done <<< "$changed"

if [ "$docs_only" = 1 ]; then echo docs; else echo code; fi
