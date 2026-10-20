#!/usr/bin/env bash
# Dependency guards for the root workspace (docs/plan/08-repo.md section 7). Exit 1 on any failure.
# Guards for crates that do not exist yet pass with a note, so the job stays green until they land.
# Needs: cargo, jq. The xtask pin-leak guard (`cargo xtask`) will cover the same rules later.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
fail=0
ok() { echo "ok:   $*"; }
bad() { echo "FAIL: $*"; fail=1; }

# absent_from <cargo tree args...>: the last arg must not be in the given graph. cargo tree exits
# 101 with "did not match any packages" when the package is not in the graph.
absent_from() {
  local out
  if out="$(cargo tree --locked --target all "$@" 2>&1)"; then
    echo "$out"
    return 1
  fi
  grep -q "did not match any packages" <<< "$out" || { echo "$out"; return 2; }
}

# 1) No tsrs in the engine graph: `cargo tree -i tsrs_core` is empty for the whole root workspace.
if absent_from --workspace -e normal,build,dev -i tsrs_core; then ok "no tsrs_core in the root workspace"; else bad "tsrs_core is in the root workspace graph (or cargo tree failed)"; fi

meta="$(cargo metadata --locked --format-version 1)" || { bad "cargo metadata --locked failed (is Cargo.lock up to date?)"; exit 1; }
members="$(jq -r '.workspace_members[]' <<< "$meta")"

# 2) Unicode pin: every unicode-id-start in the graph is >= 1.5.0 (Unicode 17).
vers="$(jq -r '.packages[] | select(.name == "unicode-id-start") | .version' <<< "$meta")"
if [ -z "$vers" ]; then
  ok "unicode-id-start not in the graph yet"
else
  for v in $vers; do
    if [ "$(printf '%s\n' 1.5.0 "$v" | sort -V | head -1)" = 1.5.0 ]; then ok "unicode-id-start $v >= 1.5.0"; else bad "unicode-id-start $v < 1.5.0"; fi
  done
fi

# 3) ep9_hints has 0 normal and build dependencies (H37).
if jq -e '.packages[] | select(.name == "ep9_hints")' <<< "$meta" > /dev/null; then
  n="$(jq '[.packages[] | select(.name == "ep9_hints") | .dependencies[] | select(.kind != "dev")] | length' <<< "$meta")"
  if [ "$n" = 0 ]; then ok "ep9_hints has 0 dependencies"; else bad "ep9_hints has $n normal/build dependencies"; fi
else
  ok "ep9_hints does not exist yet"
fi

# 4) The `object` crate has an exact `=` pin in every workspace member that uses it (H35).
reqs="$(jq -r --argjson m "$(jq -c '.workspace_members' <<< "$meta")" \
  '.packages[] | select(.id as $id | $m | index($id)) | .name as $p | .dependencies[] | select(.name == "object") | "\($p) \(.req)"' <<< "$meta")"
if [ -z "$reqs" ]; then
  ok "no member uses the object crate yet"
else
  while read -r p req; do
    if [[ "$req" == =* ]]; then ok "$p pins object $req"; else bad "$p uses object $req (needs an exact = pin)"; fi
  done <<< "$reqs"
fi

# 5) The jitless ep9 (ep9_cli without default features) links no JIT crate (H27).
if grep -q "ep9_cli" <<< "$members"; then
  for c in ep9_masm ep9_baseline ep9_opt; do
    if absent_from -p ep9_cli --no-default-features -e normal,build -i "$c"; then ok "jitless ep9 has no $c"; else bad "jitless ep9 links $c"; fi
  done
fi

exit "$fail"
