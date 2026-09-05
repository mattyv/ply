#!/usr/bin/env bash
# Builds the small site that publishes Ply's report on its own source code.
#
# In a script rather than inline in the workflow so the page can be built and
# *looked at* by hand before it is deployed:
#
#   ./.github/scripts/build-evidence-page.sh /tmp/site
#
# A page that can only be seen after a successful deploy is a page nobody
# checks, and this one exists precisely to be looked at.
#
# Reads the two drawings from the working directory and writes them, plus an
# index, into the output directory. GITHUB_SHA / RUN_URL / REPO_URL come from
# the workflow; each falls back to something sensible so a local run works.
set -euo pipefail

out=${1:?usage: build-evidence-page.sh OUT_DIR}
mkdir -p "$out"
cp ply-core-verified.svg ply-cli-verified.svg "$out/"

sha=${GITHUB_SHA:-}
repo_url=${REPO_URL:-https://github.com/mattyv/ply}
run_url=${RUN_URL:-}
built=$(date -u '+%-d %B %Y, %H:%M UTC')

if [ -n "$sha" ]; then
  origin="commit <a href=\"$repo_url/commit/$sha\"><code>${sha:0:7}</code></a>"
else
  origin="a local build"
fi
if [ -n "$run_url" ]; then
  origin="$origin &middot; <a href=\"$run_url\">the run that produced it</a>"
fi

cat > "$out/index.html" <<HTML
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Ply, checked against itself</title>
<style>
  :root { color-scheme: light dark; }
  body {
    margin: 3rem auto; padding: 0 1.5rem; max-width: 46rem;
    font: 16px/1.6 system-ui, -apple-system, Segoe UI, sans-serif;
  }
  h1 { font-size: 1.55rem; margin: 0 0 .35rem; }
  h2 { font-size: 1.1rem; margin: 3.5rem 0 .2rem; }
  .meta, .sub { opacity: .7; font-size: .9rem; }
  .sub { margin: 0 0 .5rem; }
  figure { margin: 0; }
  /* The library's drawing is 616 wide and 3494 tall. Never scale a drawing
     up to fill the column: at natural size its labels stay the size they
     were drawn to be read at. */
  img {
    max-width: 100%; height: auto;
    border: 1px solid rgba(128, 128, 128, .3); border-radius: 6px;
  }
</style>
<h1>Ply, checked against itself</h1>
<p class="meta">Built from $origin<br>$built</p>

<p>
  Ply checks whether code does what its author promised it would do. The two
  pictures below are Ply's report on Ply's own source code. They are produced
  by the build, not drawn by hand.
</p>
<p>
  Each outlined box is one part of the program, and the small chips inside it
  are its individual functions. A chip filled in <strong>green</strong> is one
  whose promise was checked on this exact commit, and held. A chip left
  <strong>grey</strong> has no evidence behind it: either nothing was run
  against it, or Ply declined to check it and said so rather than guessing.
</p>
<p>
  This page is replaced only by a build that passed, so a green chip here
  always stands for a check that really ran. When a build fails, its drawing
  is attached to that build instead and this page keeps showing the last
  state that passed.
</p>

<h2>The library</h2>
<p class="sub">
  The part that reads the promises, runs the checks, and decides what counts
  as evidence. <a href="ply-core-verified.svg">Open this drawing on its own</a>
</p>
<figure>
  <img src="ply-core-verified.svg"
       alt="Ply's library, drawn as nested boxes of functions, with each
            checked function's chip filled in green.">
</figure>

<h2>The command-line tool</h2>
<p class="sub">
  The part you actually run. <a href="ply-cli-verified.svg">Open this drawing
  on its own</a>
</p>
<figure>
  <img src="ply-cli-verified.svg"
       alt="Ply's command-line tool, drawn as nested boxes of functions, with
            each checked function's chip filled in green.">
</figure>
</html>
HTML

echo "wrote $out/index.html"
ls -la "$out"
