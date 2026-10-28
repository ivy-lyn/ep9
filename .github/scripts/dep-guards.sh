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
#     profile table and no .cargo/config.toml (or legacy .cargo/config) sets `panic`. cargo tree
#     and cargo metadata do not show profiles, so this reads the files. TOML can spell the key in
#     many ways (`"panic" =`, `'panic' =`, `release.panic =`, `release = { panic = ... }`, a
#     \u escape in a quoted key), so the scan flags every line where `panic` can be a key, and a
#     line with a quoted key that holds an escape; it may flag more than cargo reads, never less.
#     Controls: the scan must flag each spelling in a sample, must not flag `panic_x` or
#     `no-panic` keys or a comment line, and the Cargo.toml rule must accept only the one line.
# panic_keys <file>: "<table> <line>" for every such line; the table is the last [header] seen.
panic_keys() {
  awk 'BEGIN {
      q = sprintf("%c", 39)
      key = "(^|[.{,[:space:]\"" q "])panic[\"" q "]?[[:space:]]*="
      esc = "\\\\[uU][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][^\"" q "=]*[\"" q "][[:space:]]*="
    }
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*\[/ { t = $0; sub(/[[:space:]]*#.*/, "", t); gsub(/[[:space:]]/, "", t); next }
    $0 ~ key || $0 ~ esc { l = $0; sub(/^[[:space:]]+/, "", l); sub(/[[:space:]]+$/, "", l); print t " " l }' "$1"
}
# release_abort_only <keys>: the keys of Cargo.toml are exactly [profile.release] panic = "abort".
release_abort_only() {
  [ "$(grep -c . <<< "$1")" = 1 ] &&
    grep -Eq '^\[profile\.release\] panic[[:space:]]*=[[:space:]]*"abort"[[:space:]]*(#.*)?$' <<< "$1"
}
control=""
while IFS= read -r sample; do
  [ -n "$(panic_keys <(printf '%b' "$sample"))" ] || control="$control [$sample]"
done <<'SAMPLES'
[profile.dev] # x\n  panic = "unwind"  # y\n[x]\n
[profile.release]\n"panic" = "unwind"\n
[profile.release]\n'panic' = "unwind"\n
[profile]\nrelease.panic = "unwind"\n
[profile]\nrelease . "panic" = "unwind"\n
[profile]\nrelease = { panic = "unwind" }\n
[profile]\nrelease = { opt-level = 3,panic = "unwind" }\n
[profile]\nrelease = { debug = "#", panic = "unwind" }\n
[profile.release]\n"pan\\u0069c" = "unwind"\n
SAMPLES
if [ -n "$(panic_keys <(printf '[profile.release]\npanic_x = 1\nno-panic = 2\n  # panic = "unwind"\n'))" ]; then
  control="$control [panic_x, no-panic or a comment line flagged]"
fi
release_abort_only "$(panic_keys <(printf '[profile.release] # r\npanic = "abort" # H23\n'))" ||
  control="$control [the Cargo.toml rule rejects the one allowed line]"
release_abort_only "$(panic_keys <(printf '[profile.release]\npanic = "abort"\n[profile.dev]\n"panic" = "unwind"\n'))" &&
  control="$control [the Cargo.toml rule accepts a second panic key]"
if [ -z "$control" ]; then
  keys="$(panic_keys Cargo.toml)"
  if release_abort_only "$keys"; then
    ok "(13) [profile.release] has panic = \"abort\", and no other profile sets panic"
  else
    bad "(13) Cargo.toml must set panic = \"abort\" in [profile.release] and nowhere else, in no other spelling (H23); found: $(if [ -n "$keys" ]; then paste -sd ';' - <<< "$keys"; else echo "no panic key"; fi)"
  fi
  for f in .cargo/config.toml .cargo/config; do
    if [ -f "$f" ] && [ -n "$(panic_keys "$f")" ]; then
      bad "(13) $f sets panic: $(panic_keys "$f" | paste -sd ';' -)"
    else
      ok "(13) $f sets no panic"
    fi
  done
else
  bad "(13) control: the panic scan failed on samples:$control"
fi

# 14) ep9_regexp is reached only from ep9_vm (08-repo section 2; decisions-log M4-PLAN-D15):
#     no other workspace member has an edge to it, of any kind (normal, build or dev), so
#     builtins reach the RegExp backend only through Cx methods. Check 8 keeps ep9_builtins'
#     own edges. Control: the same query on a sample graph with an ep9_builtins -> ep9_regexp
#     edge must name ep9_builtins, so a query that never matches fails here.
# dependents_of <name> <metadata json>: the workspace members with a direct edge to <name>.
dependents_of() {
  jq -r --arg n "$1" '.workspace_members as $m | .packages[] | select(.id as $id | $m | index($id)) | select(any(.dependencies[]; .name == $n)) | .name' <<< "$2" | sort -u
}
sample='{"workspace_members":["b","v","r"],"packages":[{"id":"b","name":"ep9_builtins","dependencies":[{"name":"ep9_vm"},{"name":"ep9_regexp"}]},{"id":"v","name":"ep9_vm","dependencies":[{"name":"ep9_regexp"}]},{"id":"r","name":"ep9_regexp","dependencies":[]}]}'
if [ "$(dependents_of ep9_regexp "$sample" | paste -sd ' ' -)" != "ep9_builtins ep9_vm" ]; then
  bad "(14) control: the query does not find ep9_builtins -> ep9_regexp in a sample graph"
elif ! member_exists ep9_regexp; then
  ok "(14) ep9_regexp does not exist yet"
else
  users="$(dependents_of ep9_regexp "$meta" | paste -sd ' ' -)"
  if [ "$users" = ep9_vm ]; then ok "(14) only ep9_vm depends on ep9_regexp"; else bad "(14) ep9_regexp must have ep9_vm as its only dependent; it has: ${users:-none}"; fi
fi

# 15) ep9_cli: only the `reprl` module opts out of `#![deny(unsafe_code)]` (plan v9 V9-A3; a
#     crate-level `forbid` cannot be relaxed for one module, so the crate uses `deny`, which any
#     later `allow` would relax). Every line of crates/ep9_cli/src that names `unsafe_code` must be
#     main.rs `#![deny(unsafe_code)]` or main.rs `#[allow(unsafe_code)]` followed by `mod reprl;`.
#     Controls: the rule accepts those two lines and rejects a sample with one more allow.
# unsafe_code_lines <files...>: "<file>: <line>" per line that names unsafe_code; after an
# allow, expect or warn line, " => <next line>" is added.
unsafe_code_lines() {
  awk '/unsafe_code/ { l = $0; gsub(/^[[:space:]]+|[[:space:]]+$/, "", l); out = FILENAME ": " l
      if (l ~ /allow|expect|warn/ && (getline n) > 0) { gsub(/^[[:space:]]+|[[:space:]]+$/, "", n); out = out " => " n }
      print out }' "$@"
}
cli_unsafe_ok() {
  [ "$1" = "crates/ep9_cli/src/main.rs: #![deny(unsafe_code)]
crates/ep9_cli/src/main.rs: #[allow(unsafe_code)] => mod reprl;" ]
}
good='crates/ep9_cli/src/main.rs: #![deny(unsafe_code)]
crates/ep9_cli/src/main.rs: #[allow(unsafe_code)] => mod reprl;'
sample="$(unsafe_code_lines <(printf '#[allow(dead_code, unsafe_code)]\nfn f() {}\n'))"
if ! cli_unsafe_ok "$good" || cli_unsafe_ok "$good
$sample" || [ "${sample#*: }" != '#[allow(dead_code, unsafe_code)] => fn f() {}' ]; then
  bad "(15) control: the ep9_cli unsafe_code rule does not accept the 2 lines or does not reject one more allow"
else
  lines="$(unsafe_code_lines $(find crates/ep9_cli/src -name '*.rs' | LC_ALL=C sort))"
  if cli_unsafe_ok "$lines"; then
    ok "(15) ep9_cli denies unsafe_code; only mod reprl allows it"
  else
    bad "(15) ep9_cli: only main.rs '#![deny(unsafe_code)]' and '#[allow(unsafe_code)] mod reprl;' may name unsafe_code; found: $(paste -sd ';' - <<< "$lines")"
  fi
fi

# 16) The regress fork is live (V9-D15; third_party/patches/regress/README.md). A [patch] table
#     applies only at its own workspace root, so the root, spikes/spike-b and spikes/spike-c each
#     have a `[patch.crates-io] regress` path entry. In each of the 3 workspaces `cargo metadata
#     --locked` must show regress, and every regress package must have no source (a path
#     package) and the manifest third_party/regress/Cargo.toml (scripts/sync-regress.sh makes
#     it). Without the entry regress comes from crates.io (or the lock no longer matches) and
#     this fails. Controls: a sample graph with a crates.io regress must fail the rule and a
#     sample with the path package must pass it, so a rule that always (or never) passes fails.
# fork_live <metadata json>: 0 when the graph has regress and each regress is the path package.
fork_live() {
  local rows
  rows="$(jq -r '.packages[] | select(.name == "regress") | "\(.source // "path")|\(.manifest_path)"' <<< "$1")" || return 1
  [ -n "$rows" ] || return 1
  while IFS='|' read -r src mp; do
    [ "$src" = path ] && [[ "$mp" == */third_party/regress/Cargo.toml ]] || return 1
  done <<< "$rows"
}
reg='{"packages":[{"name":"memchr","source":"registry+https://github.com/rust-lang/crates.io-index","manifest_path":"/c/registry/src/memchr-2.8.3/Cargo.toml"},{"name":"regress","source":"registry+https://github.com/rust-lang/crates.io-index","manifest_path":"/c/registry/src/regress-0.12.0/Cargo.toml"}]}'
fork='{"packages":[{"name":"regress","source":null,"manifest_path":"/w/third_party/regress/Cargo.toml"}]}'
if fork_live "$reg" || ! fork_live "$fork" || fork_live '{"packages":[]}'; then
  bad "(16) control: the fork rule passes a crates.io regress or an empty graph, or fails the path package, in a sample graph"
else
  for m in Cargo.toml spikes/spike-b/Cargo.toml spikes/spike-c/Cargo.toml; do
    if ! wmeta="$(cargo metadata --locked --format-version 1 --manifest-path "$m")"; then
      bad "(16) cargo metadata --locked --manifest-path $m failed (run scripts/sync-regress.sh first; is the lock up to date?)"
    elif fork_live "$wmeta"; then
      ok "(16) $m: regress is the fork checkout third_party/regress (path package, no registry source)"
    else
      found="$(jq -r '.packages[] | select(.name == "regress") | "\(.source // "path") \(.manifest_path)"' <<< "$wmeta" | paste -sd ';' -)"
      bad "(16) $m: regress must be only the third_party/regress path package; found: ${found:-no regress}"
    fi
  done
fi

exit "$fail"
