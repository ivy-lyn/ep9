#!/usr/bin/env bash
# The ep9-typecheck lock guard (H26:42; docs/plan/08-repo.md section 7): dep-guards check 27
# (the required dependency guards job), and the sp-tsrs-win job of ci.yml after its tsrs sync.
# usage: bash tools/ep9-typecheck/lock-guard.sh [<tsrs Cargo.lock> <ep9-typecheck Cargo.lock>]
#        (default: third_party/tsrs/Cargo.lock and tools/ep9-typecheck/Cargo.lock of this repo)
#
# scripts/sync-tsrs.sh step 2 makes the ep9-typecheck lock: a copy of the tsrs lock, then
# `cargo update --workspace`, which adds the 2 ep9 packages and drops the tsrs packages that the
# tool does not use. PASS when each package of the ep9-typecheck lock is
#   - a package of the tsrs lock with the same name, version, source and checksum, or
#   - ep9-typecheck or ep9_hints: a path package (no source) that the tsrs lock does not have,
#     each exactly once.
# Anything else FAILs and names the package: an extra one, or a tsrs package with another
# version, source or checksum. A line that is not in the form that cargo writes FAILs too.
# Either lock absent: FAIL in CI (CI=true; the dep-guards job makes both locks first, with no
# build, so the guard compares on every PR); a PASS with a note only outside CI. Exit 1 on FAIL,
# 2 on bad arguments.
set -euo pipefail
export LC_ALL=C
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
case $# in
  0) tl="$root/third_party/tsrs/Cargo.lock" el="$root/tools/ep9-typecheck/Cargo.lock" ;;
  2) tl=$1 el=$2 ;;
  *) echo "usage: bash tools/ep9-typecheck/lock-guard.sh [<tsrs Cargo.lock> <ep9-typecheck Cargo.lock>]" >&2; exit 2 ;;
esac
fail=0
bad() { echo "FAIL: (27) $*"; fail=1; }

missing=""
[ -f "$tl" ] || missing="$tl"
[ -f "$el" ] || missing="${missing:+$missing and }$el"
if [ -n "$missing" ] && [ "${CI:-}" = true ]; then
  bad "the ep9-typecheck lock guard cannot run in CI: $missing absent (dep-guards.sh check 27 and scripts/sync-tsrs.sh make both)"
  exit 1
elif [ -n "$missing" ]; then
  echo "PASS: (27) the ep9-typecheck lock guard did not run: $missing absent (scripts/sync-tsrs.sh makes both)"
  exit 0
fi

# form <lock>: "N|text" for the first line that is not in the form that cargo writes, else nothing.
# The parser below reads only that form, so any other line (a header with a comment, a key with
# no spaces, another table) FAILs: no valid TOML entry can stay out of the comparison.
form() {
  awk '{ sub(/\r$/, "") }
    !/^(\[\[package\]\]|name = "[^"]+"|version = ("[^"]+"|[0-9]+)|source = "[^"]+"|checksum = "[0-9a-f]+"|dependencies = \[\]?| "[^"]+",|\]|# .*)?$/ {
      print NR "|" $0; exit
    }' "$1"
}
for l in "$tl" "$el"; do
  b="$(form "$l")"
  [ -z "$b" ] || bad "line ${b%%|*} of $l is not in the Cargo lock form: ${b#*|}"
done
[ "$fail" = 0 ] || exit 1

# pkgs <lock>: one line per [[package]] entry, "name|version|source|checksum", sorted; a path
# package has an empty source and checksum.
pkgs() {
  awk '
    function flush() {
      if (inpkg && name != "") print name "|" ver "|" src "|" sum
      inpkg = 0; name = ""; ver = ""; src = ""; sum = ""
    }
    { sub(/\r$/, "") }
    /^\[\[package\]\]$/ { flush(); inpkg = 1; next }
    /^\[/ { flush(); next }
    inpkg && /^name = / { name = $3 }
    inpkg && /^version = / { ver = $3 }
    inpkg && /^source = / { src = $3 }
    inpkg && /^checksum = / { sum = $3 }
    END { flush() }' "$1" | tr -d '"' | sort -u
}
t="$(pkgs "$tl")"
e="$(pkgs "$el")"
[ -n "$t" ] || bad "no package in $tl"
[ -n "$e" ] || bad "no package in $el"
[ "$fail" = 0 ] || exit 1
tnames="$(cut -d'|' -f1 <<< "$t" | sort -u)"

# The entries of the ep9-typecheck lock that the tsrs lock does not have, field for field.
while IFS='|' read -r n v s c; do
  [ -n "$n" ] || continue
  if grep -qxF -- "$n" <<< "$tnames"; then
    have="$(grep -- "^$n|" <<< "$t" | cut -d'|' -f2,3 | tr '|\n' ' ;' | sed 's/; *$//')"
    bad "changed: $n $v ${s:-(path)}${c:+ checksum $c} is not the tsrs lock entry (tsrs: $have)"
  elif [ "$n" != ep9-typecheck ] && [ "$n" != ep9_hints ]; then
    bad "extra: $n $v ${s:-(path)} is not in the tsrs lock (only ep9-typecheck and ep9_hints may be added)"
  elif [ -n "$s" ] || [ -n "$c" ]; then
    bad "$n $v must be a path package, not $s"
  fi
done <<< "$(comm -13 <(printf '%s\n' "$t") <(printf '%s\n' "$e"))"
for n in ep9-typecheck ep9_hints; do
  k="$(grep -c -- "^$n|" <<< "$e" || true)"
  [ "$k" = 1 ] || bad "$n is in $el $k times (needs exactly 1)"
done
if [ "$fail" = 0 ]; then
  echo "PASS: (27) the ep9-typecheck lock ($(wc -l <<< "$e" | tr -d ' ') packages) is the tsrs lock ($(wc -l <<< "$t" | tr -d ' ') packages) plus only ep9-typecheck and ep9_hints"
fi
exit "$fail"
