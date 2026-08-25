"""The one-line overflow fix `n1`'s own baseline earns, applied to a scratch copy.

Identical in kind to vetting/004-legacy-extension/run.sh::apply_overflow_fix:
widen the product to u64 before dividing. Kept out of the committed fixture so
`natural/feature/src/lib.rs` stays exactly as it was written and hashed before
the first Kani run.
"""
import sys

p = sys.argv[1]
t = open(p).read()
old = "    let vat = net_cents * catalog::vat_bps(region) / 10_000;"
new = "    let vat = ((net_cents as u64 * catalog::vat_bps(region) as u64) / 10_000) as u32;"
if old not in t:
    sys.exit("OVERFLOW FIX DID NOT APPLY")
open(p, "w").write(t.replace(old, new))
