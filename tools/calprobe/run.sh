#!/usr/bin/env bash
# calprobe gate (INTL-I1-D2, H36 fp-check entry 2): 0 core_maths calls and 0 astronomy calls on
# the calendar paths that ep9 reaches, and a positive control that counts them.
#   tools/calprobe/run.sh            all modes; exit 0 only if every expectation holds
# Needs: cargo, python3 (3.11+). Offline when the registry has the crates (the root build fetched
# them). Writes only under $CALPROBE_WORK (default tools/calprobe/target/work).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
work="${CALPROBE_WORK:-$here/target/work}"
py() { python3 -I "$@"; }

# 1. The probe resolves the versions that ep9 builds.
py "$here/lockcheck.py" "$here/Cargo.lock" "$root/Cargo.lock"

# 2. Counting copies of core_maths and calendrical_calculations from the registry sources.
rm -rf "$work"
mkdir -p "$work"
meta="$work/metadata.json"
cargo metadata --manifest-path "$here/Cargo.toml" --locked --offline --format-version 1 > "$meta"
for name in core_maths calendrical_calculations; do
  dir="$(py -c 'import json,os,sys; m=json.load(open(sys.argv[1])); print(next(os.path.dirname(p["manifest_path"]) for p in m["packages"] if p["name"]==sys.argv[2]))' "$meta" "$name")"
  cp -R "$dir" "$work/$name"
  chmod -R u+w "$work/$name"
  py "$here/instr.py" "$work/$name"
done

# 3. The probe crate with the copies patched in (a copy, so the tracked files do not change),
#    and icu_calendar from the patched checkout that ep9 builds (scripts/sync-icu4x.sh; the
#    root [patch.crates-io] entry; INTL W1 R2), so the probe counts the code that ep9 ships.
[ -f "$root/third_party/icu4x/components/calendar/Cargo.toml" ] \
  || { echo "calprobe: run scripts/sync-icu4x.sh first (third_party/icu4x is missing)" >&2; exit 1; }
mkdir -p "$work/probe"
cp -R "$here/src" "$here/Cargo.toml" "$here/Cargo.lock" "$work/probe/"
cat >> "$work/probe/Cargo.toml" <<TOML

[patch.crates-io]
core_maths = { path = "../core_maths" }
calendrical_calculations = { path = "../calendrical_calculations" }
icu_calendar = { path = "$root/third_party/icu4x/components/calendar" }
TOML
export CARGO_TARGET_DIR="$work/target"
(cd "$work/probe" && cargo build --release --offline --features instr,control --quiet)
# The patch must not move a version: the copies have the registry versions.
py "$here/lockcheck.py" "$work/probe/Cargo.lock" "$root/Cargo.lock"
bin="$work/target/release/calprobe"

# 4. Run and check. icu and temporal: 0 and 0. control: every count above 0.
fail=0
for mode in icu temporal control; do
  "$bin" "$mode" | tee "$work/out-$mode.txt"
  line="$(grep "^TOTAL $mode " "$work/out-$mode.txt")"
  m="$(sed -E 's/.* core_maths=([0-9]+).*/\1/' <<< "$line")"
  a="$(sed -E 's/.* astronomy=([0-9]+).*/\1/' <<< "$line")"
  s="$(sed -E 's/.* sin=([0-9]+).*/\1/' <<< "$line")"
  if [ "$mode" = control ]; then
    if [ "$m" -gt 0 ] && [ "$a" -gt 0 ] && [ "$s" -gt 0 ]; then
      echo "PASS: control counts core_maths=$m astronomy=$a sin=$s (the counters work)"
    else
      echo "FAIL: control counts core_maths=$m astronomy=$a sin=$s (a counter does not count)"; fail=1
    fi
  elif [ "$m" = 0 ] && [ "$a" = 0 ]; then
    echo "PASS: $mode core_maths=0 astronomy=0"
  else
    echo "FAIL: $mode core_maths=$m astronomy=$a (a runtime path reaches libm or astronomy)"; fail=1
  fi
done
exit "$fail"
