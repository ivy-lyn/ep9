#!/usr/bin/env bash
# Dependency guards for the root workspace (docs/plan/08-repo.md section 7). Exit 1 on any failure.
# Guards for crates that do not exist yet pass with a note, so the job stays green until they land.
# Needs: cargo, jq. The xtask pin-leak guard (`cargo xtask`) will cover the same rules later.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
fail=0
ok() { echo "PASS: $*"; }
bad() { echo "FAIL: $*"; fail=1; }

# 0) Every directory that a `members` glob of Cargo.toml matches (hidden ones too) has a
#    Cargo.toml or is under an `exclude` path. Without one, every cargo command fails with
#    "failed to load manifest for workspace member", so this runs first and stops on failure.
# toml_array <key>: the strings of the array `<key> = [...]` in Cargo.toml (one line or several).
toml_array() {
  awk -v k="$1" '$0 ~ "^" k " *= *\\[" { on = 1 } on { sub(/#.*/, ""); print; if (/\]/) exit }' Cargo.toml |
    grep -o '"[^"]*"' | tr -d '"' || true
}
globs="$(toml_array members)"
excludes="$(toml_array exclude)"
if [ -z "$globs" ]; then
  bad "(0) cannot read the members array of Cargo.toml"
  exit 1
fi
shopt -s nullglob dotglob
while IFS= read -r g; do
  for d in $g; do
    [ -d "$d" ] || continue
    [ -f "$d/Cargo.toml" ] && continue
    skip=0
    while IFS= read -r e; do
      if [ -n "$e" ] && { [ "$d" = "$e" ] || [[ "$d" == "$e"/* ]]; }; then skip=1; fi
    done <<< "$excludes"
    [ "$skip" = 1 ] || bad "(0) $d matches the member glob \"$g\" but has no Cargo.toml: add one, or add the directory to exclude in Cargo.toml"
  done
done <<< "$globs"
shopt -u nullglob dotglob
if [ "$fail" = 1 ]; then
  echo "FAIL: (0) later checks need a loadable workspace; stopping"
  exit 1
fi
ok "(0) every directory under the member globs has a Cargo.toml or is excluded"

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

# features_of <package> <cargo tree args...>: the features that <package> has in that graph, one
# line per package instance, comma-separated (an empty line means no features). No output when
# <package> is not in the graph; exit 2 when cargo tree fails. Dev edges are included, so the
# graph is the one a test build of the selected package resolves; the shipped binary's graph is
# a subset of it.
features_of() {
  local pkg="$1" out
  shift
  out="$(cargo tree --locked --target all "$@" -e normal,build,dev --prefix none -f '{p}|{f}')" ||
    { echo "cargo tree $* failed" >&2; return 2; }
  awk -F'|' -v p="$pkg v" 'index($1, p) == 1 { sub(/ \(\*\)$/, "", $2); print $2 }' <<< "$out" | sort -u
}

# has_feature <features_of output> <feature>
has_feature() {
  local s=",${1//$'\n'/,},"
  [[ "$s" == *",$2,"* ]]
}

# feature_check <check> <package> <feature> <want: on|off> <label> <cargo tree args...>
feature_check() {
  local check="$1" pkg="$2" feat="$3" want="$4" label="$5" f
  shift 5
  if ! f="$(features_of "$pkg" "$@")"; then
    bad "($check) cargo tree $* failed"
  elif has_feature "$f" "$feat"; then
    if [ "$want" = on ]; then ok "($check) $label has $pkg feature $feat"; else bad "($check) $label turns on $pkg feature $feat"; fi
  else
    if [ "$want" = off ]; then ok "($check) $label has no $pkg feature $feat"; else bad "($check) $label does not show $pkg feature $feat"; fi
  fi
}

# 1) No tsrs in the engine graph: `cargo tree -i tsrs_core` is empty for the whole root workspace.
if absent_from --workspace -e normal,build,dev -i tsrs_core; then ok "(1) no tsrs_core in the root workspace"; else bad "(1) tsrs_core is in the root workspace graph (or cargo tree failed)"; fi

meta="$(cargo metadata --locked --format-version 1)" || { bad "cargo metadata --locked failed (is Cargo.lock up to date?)"; exit 1; }
members="$(jq -r '.workspace_members[]' <<< "$meta")"

# member_exists <name>, member_deps <name>: a workspace member and the package names of its
# normal and build dependencies (dev-dependencies are left out), one per line.
member_exists() {
  jq -e --arg n "$1" '.workspace_members as $m | .packages[] | select(.name == $n and (.id as $id | $m | index($id)))' <<< "$meta" > /dev/null
}
member_deps() {
  jq -r --arg n "$1" '.workspace_members as $m | .packages[] | select(.name == $n and (.id as $id | $m | index($id))) | .dependencies[] | select(.kind != "dev") | .name' <<< "$meta" | sort -u
}

# deps_within <check> <crate> <allowed...>: the crate's normal and build dependencies are a
# subset of the allowed names (an empty list allows none).
deps_within() {
  local check="$1" crate="$2" extra
  shift 2
  if ! member_exists "$crate"; then ok "($check) $crate does not exist yet"; return; fi
  extra="$(member_deps "$crate" | grep -vxF -f <(printf '%s\n' "$@") | paste -sd ' ' - || true)"
  if [ -z "$extra" ] && [ $# = 0 ]; then
    ok "($check) $crate has 0 normal/build dependencies"
  elif [ -z "$extra" ]; then
    ok "($check) $crate normal/build dependencies are within {$(IFS=,; echo "$*")}"
  elif [ $# = 0 ]; then
    bad "($check) $crate must have 0 normal/build dependencies; it has: $extra"
  else
    bad "($check) $crate has normal/build dependencies outside {$(IFS=,; echo "$*")}: $extra"
  fi
}

# 2) Unicode pin: every unicode-id-start in the graph is >= 1.5.0 (Unicode 17).
vers="$(jq -r '.packages[] | select(.name == "unicode-id-start") | .version' <<< "$meta")"
if [ -z "$vers" ]; then
  ok "(2) unicode-id-start not in the graph yet"
else
  for v in $vers; do
    if [ "$(printf '%s\n' 1.5.0 "$v" | sort -V | head -1)" = 1.5.0 ]; then ok "(2) unicode-id-start $v >= 1.5.0"; else bad "(2) unicode-id-start $v < 1.5.0"; fi
  done
fi

# 3) ep9_hints has 0 normal and build dependencies (H37).
deps_within 3 ep9_hints

# 4) The `object` crate has an exact `=` pin in every workspace member that uses it (H35).
reqs="$(jq -r --argjson m "$(jq -c '.workspace_members' <<< "$meta")" \
  '.packages[] | select(.id as $id | $m | index($id)) | .name as $p | .dependencies[] | select(.name == "object") | "\($p) \(.req)"' <<< "$meta")"
if [ -z "$reqs" ]; then
  ok "(4) no member uses the object crate yet"
else
  while read -r p req; do
    if [[ "$req" == =* ]]; then ok "(4) $p pins object $req"; else bad "(4) $p uses object $req (needs an exact = pin)"; fi
  done <<< "$reqs"
fi

# 5) The jitless ep9 (ep9_cli without default features) links no JIT crate (H27).
if grep -q "ep9_cli" <<< "$members"; then
  for c in ep9_masm ep9_baseline ep9_opt; do
    if absent_from -p ep9_cli --no-default-features -e normal,build -i "$c"; then ok "(5) jitless ep9 has no $c"; else bad "(5) jitless ep9 links $c"; fi
  done
fi

# 6) ep9_math has 0 normal and build dependencies (H36).
deps_within 6 ep9_math

# 7) ep9_facts depends on ep9_bytecode only (H16: a fact holds no VM or heap type).
deps_within 7 ep9_facts ep9_bytecode

# 8) ep9_builtins reaches the engine only through ep9_vm, plus ep9_math for Math (H20).
deps_within 8 ep9_builtins ep9_vm ep9_math

# 9) The jitless ep9 has no `jit` feature anywhere in its graph (H27): no ep9_platform "jit".
#    Control: the default ep9 must show it, so a broken jit chain or a cargo output change that
#    hides features fails here instead of passing silently.
if grep -q "ep9_cli" <<< "$members"; then
  feature_check 9 ep9_platform jit on "control: the default ep9 (-p ep9_cli)" -p ep9_cli
  feature_check 9 ep9_platform jit off "the jitless ep9 (-p ep9_cli --no-default-features)" -p ep9_cli --no-default-features
fi

# 10) ep9_unit's `aot` feature (the unit writer) is never in the ep9 graph, default or jitless
#     (H27). Control: `-p ep9_unit --features aot` must show it.
if member_exists ep9_unit; then
  feature_check 10 ep9_unit aot on "control: -p ep9_unit --features aot" -p ep9_unit --features aot
  if grep -q "ep9_cli" <<< "$members"; then
    feature_check 10 ep9_unit aot off "the default ep9 (-p ep9_cli)" -p ep9_cli
    feature_check 10 ep9_unit aot off "the jitless ep9 (-p ep9_cli --no-default-features)" -p ep9_cli --no-default-features
  fi
else
  ok "(10) ep9_unit does not exist yet"
fi

# 11) The ep9_vm features `spike-native` (Spike B's native entry) and `ic-stats` (IC counters)
#     are never in the ep9 graph, default or jitless (plan M0 month 2, rule 11; H27). Control:
#     `-p ep9_vm --features spike-native,ic-stats` must show both.
if grep -q "ep9_cli" <<< "$members" && member_exists ep9_vm; then
  for feat in spike-native ic-stats; do
    feature_check 11 ep9_vm "$feat" on "control: -p ep9_vm --features spike-native,ic-stats" -p ep9_vm --features spike-native,ic-stats
    feature_check 11 ep9_vm "$feat" off "the default ep9 (-p ep9_cli)" -p ep9_cli
    feature_check 11 ep9_vm "$feat" off "the jitless ep9 (-p ep9_cli --no-default-features)" -p ep9_cli --no-default-features
  done
fi

# 12) H5: the bytecode compiler is always linked. ep9 (the runtime crate, ep9_cli for now) has
#     ep9_frontend in its normal graph, default and jitless; eval reaches it through the
#     IsolateConfig::compiler hook. Control: ep9_vm's graph must not show ep9_frontend (ep9_vm
#     has no edge to it), so a query that always matches fails here instead of passing.
# present_in <package> <cargo tree args... -i <dep>>: <package> reaches <dep> in the given graph.
# cargo tree must exit 0, and its stdout (the inverted tree, one node per line with
# --prefix none) must hold a "<package> v" node. Only stdout is read: for a graph without <dep>
# on those edges cargo tree exits 0 with "warning: nothing to print." on stderr and no stdout.
present_in() {
  local pkg="$1" out
  shift
  out="$(cargo tree --locked --target all --prefix none -p "$pkg" "$@")" || return 1
  grep -q "^$pkg v" <<< "$out"
}
if grep -q "ep9_cli" <<< "$members"; then
  if absent_from -p ep9_vm -e normal,build -i ep9_frontend; then ok "(12) control: ep9_vm has no ep9_frontend"; else bad "(12) control: ep9_vm reaches ep9_frontend (or cargo tree failed)"; fi
  # Control for present_in: ep9_builtins has ep9_frontend only as a dev-dependency (check 8
  # keeps its normal edges to ep9_vm and ep9_math), so on normal edges cargo tree prints
  # nothing and present_in must say no.
  if present_in ep9_builtins -e normal -i ep9_frontend 2>/dev/null; then bad "(12) control: present_in accepts ep9_builtins -> ep9_frontend on normal edges (a dev-only edge)"; else ok "(12) control: present_in rejects a dev-only edge (ep9_builtins -e normal -i ep9_frontend)"; fi
  if present_in ep9_cli -e normal -i ep9_frontend; then ok "(12) the default ep9 (-p ep9_cli) links ep9_frontend"; else bad "(12) the default ep9 (-p ep9_cli) does not link ep9_frontend"; fi
  if present_in ep9_cli --no-default-features -e normal -i ep9_frontend; then ok "(12) the jitless ep9 (-p ep9_cli --no-default-features) links ep9_frontend"; else bad "(12) the jitless ep9 does not link ep9_frontend"; fi
fi

# 13) H23: release builds abort on a panic, so no panic unwinds through a JIT or AOT frame.
#     [profile.release] of Cargo.toml has `panic = "abort"` (dev-opt inherits it), and no other
#     profile table and no .cargo/config.toml sets `panic`. cargo tree and cargo metadata do not
#     show profiles, so this reads the files. Control: the scan must find a `panic` key in a
#     sample with one, so a scan that never matches fails here instead of passing.
# panic_keys <file>: "<table> <value>" for every `panic = <value>` line, comments dropped.
panic_keys() {
  awk '{ sub(/[[:space:]]*#.*/, "") }
    /^[[:space:]]*\[/ { t = $0; gsub(/[[:space:]]/, "", t); next }
    /^[[:space:]]*panic[[:space:]]*=/ { v = $0; sub(/^[^=]*=[[:space:]]*/, "", v); sub(/[[:space:]]+$/, "", v); print t " " v }' "$1"
}
sample="$(panic_keys <(printf '[profile.dev] # x\n  panic = "unwind"  # y\n[x]\n'))"
if [ "$sample" = '[profile.dev] "unwind"' ]; then
  keys="$(panic_keys Cargo.toml)"
  if [ "$keys" = '[profile.release] "abort"' ]; then
    ok "(13) [profile.release] has panic = \"abort\", and no other profile sets panic"
  else
    bad "(13) Cargo.toml must set panic = \"abort\" in [profile.release] and nowhere else (H23); found: $(if [ -n "$keys" ]; then paste -sd ';' - <<< "$keys"; else echo "no panic key"; fi)"
  fi
  if [ -f .cargo/config.toml ] && [ -n "$(panic_keys .cargo/config.toml)" ]; then
    bad "(13) .cargo/config.toml sets panic: $(panic_keys .cargo/config.toml | paste -sd ' ' -)"
  else
    ok "(13) .cargo/config.toml sets no panic"
  fi
else
  bad "(13) control: the panic scan read '$sample' from a sample with [profile.dev] panic = \"unwind\""
fi

exit "$fail"
