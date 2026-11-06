#!/usr/bin/env bash
# Remakes the Verdict-row fixtures of this directory with the producer, `spike-c verdict-rows`
# (spikes/spike-c/src/verdict.rs; plan v11 WI-SC15, X18): the m1-exit item 7 and 10 tests read
# the table in the form the producer prints. Run from the repo root:
#   bash tools/xtask/tests/m1_exit/make-verdict-rows.sh          (writes the two files)
#   bash tools/xtask/tests/m1_exit/make-verdict-rows.sh --check  (compares, writes nothing)
# The inputs are the producer's own fixture inputs (`spike-c verdict-fixtures`, the files of
# `gate.sh verdict --fixtures`) with one change: the commit is the placeholder `c0` x 20, which
# the tests replace with the fake repo's HEAD.
#   verdict-rows.md           C4 navier-stokes moved to the pass side (R [3.320, 3.505]), so
#                             every row is `go`
#   verdict-rows-marginal.md  the fixture inputs as written: C4 navier-stokes `marginal`
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
check=0
[ "${1:-}" = "--check" ] && check=1
commit="$(printf 'c0%.0s' $(seq 20))"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
spike=(cargo run --quiet --locked --manifest-path spikes/spike-c/Cargo.toml --)
"${spike[@]}" verdict-fixtures "$tmp/in"
sed -i.bak "s/^commit=.*/commit=$commit/" "$tmp/in/meta.txt"
rm -f "$tmp/in/meta.txt.bak"
"${spike[@]}" verdict-rows "$tmp/in" >"$tmp/verdict-rows-marginal.md"
sed -i.bak 's/median 3\.010, interval \[2\.950, 3\.120\], IQR 2\.0% of the median; vs 3: marginal/median 3.410, interval [3.320, 3.505], IQR 2.0% of the median; vs 3: interval on the pass side/' \
  "$tmp/in/c4-navier-stokes.txt"
rm -f "$tmp/in/c4-navier-stokes.txt.bak"
grep -q 'interval \[3.320, 3.505\]' "$tmp/in/c4-navier-stokes.txt" ||
  { echo "make-verdict-rows: the navier-stokes fixture line changed; update this script" >&2; exit 1; }
"${spike[@]}" verdict-rows "$tmp/in" >"$tmp/verdict-rows.md"
for f in verdict-rows.md verdict-rows-marginal.md; do
  if [ $check = 1 ]; then
    diff -u "$here/$f" "$tmp/$f" || { echo "make-verdict-rows: $f differs from the producer output" >&2; exit 1; }
  else
    cp "$tmp/$f" "$here/$f"
  fi
done
[ $check = 1 ] && echo "make-verdict-rows: both fixtures equal the producer output"
exit 0
