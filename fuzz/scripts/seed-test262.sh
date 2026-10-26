#!/usr/bin/env bash
# Writes a seed corpus for the 3 front-end targets from test262 sources (the checkout of
# scripts/sync-test262.sh), in each target's input form:
#
#   frontend_compile/t262-NNNNN   goal byte (0 sloppy, 1 strict for onlyStrict, 2 module) + text
#   function_ctor/t262-NNNNN      kind byte 0 + NUL + text (empty params, the text as the body;
#                                 module files skipped)
#   eval_context/t262-NNNNN       2 context bytes (direct; + strict for onlyStrict) + text
#                                 (module files skipped)
#
# Files: test/language/**/*.js below 4 KiB (the default -max_len of run.sh), no *_FIXTURE.js,
# sorted by path (LC_ALL=C), then every k-th so that at most <max> stay (default 1500). The
# YAML front matter and the leading comment lines are cut, so a seed is mostly code. Same
# checkout -> same corpus.
#
#   fuzz/scripts/seed-test262.sh <test262 dir> <corpus root> [<max>]
set -euo pipefail
[ $# -ge 2 ] || { echo "usage: $0 <test262 dir> <corpus root> [<max>]" >&2; exit 2; }
t262="$(cd "$1" && pwd)"
out="$2"
max="${3:-1500}"
[ -d "$t262/test/language" ] || { echo "no test/language in $t262" >&2; exit 2; }
mkdir -p "$out/frontend_compile" "$out/function_ctor" "$out/eval_context"

list="$(mktemp)"
trap 'rm -f "$list"' EXIT
(cd "$t262/test" && find language -type f -name '*.js' ! -name '*_FIXTURE.js' -size -4k) |
  LC_ALL=C sort >"$list"
total=$(wc -l <"$list" | tr -d ' ')
step=$(((total + max - 1) / max))
[ "$step" -ge 1 ] || step=1

i=0
kept=0
while IFS= read -r rel; do
  i=$((i + 1))
  [ $(((i - 1) % step)) -eq 0 ] || continue
  kept=$((kept + 1))
  src="$t262/test/$rel"
  name="$(printf 't262-%05d' "$kept")"
  body="$(mktemp)"
  perl -0777 -pe 's{/\*---.*?---\*/}{}s; s{\A(?:[ \t]*//[^\n]*\n|[ \t]*\n)+}{}' "$src" >"$body"
  goal=0
  if grep -qE '^flags:.*\bmodule\b' "$src"; then
    goal=2
  elif grep -qE '^flags:.*\bonlyStrict\b' "$src"; then
    goal=1
  fi
  { printf "\\$(printf '%03o' "$goal")"; cat "$body"; } >"$out/frontend_compile/$name"
  if [ "$goal" -ne 2 ]; then
    { printf '\000\000'; cat "$body"; } >"$out/function_ctor/$name"
    # Context bits, little endian: bit 1 direct, bit 0 strict.
    { printf "\\$(printf '%03o' $((2 + (goal == 1 ? 1 : 0))))\\000"; cat "$body"; } \
      >"$out/eval_context/$name"
  fi
  rm -f "$body"
done <"$list"
echo "seed-test262: $kept of $total files (every ${step}th) -> $out/{frontend_compile,function_ctor,eval_context}"
