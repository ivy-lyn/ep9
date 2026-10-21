#!/usr/bin/env bash
# Spike B D21 bench (docs/measurements/2026-10-22-spike-b-protocol.md; owner P3-SPIKEB-BENCH).
# It runs the prebuilt release binary and builds nothing (protocol §3, "same session").
#
#   bench.sh              the D21 run (protocol §3-§9):
#                           1 tau from the SP-Dispatch bench.json of a gate.sh run just before
#                             (median_ns.sibling / dispatches of kernel loop; release = sibling,
#                             scale = 1, results_equal = true, else it stops)
#                           2 the load guard before (tools/bench/lib/host.mjs guardBefore) and the
#                             gate-host check (tools/bench/hosts/<host>.toml)
#                           3 spike-b bench: k, 1 warm-up + 15 kept rounds of A, C, A0, J, JC (one
#                             process per run), RT_r, median, interval, A0 and J reports; a
#                             marginal result re-runs with 31 kept rounds
#                           4 the load guard after (guardAfter, 120 s), then the verdict
#   bench.sh cachegrind   the instruction ratio of protocol §8 (a report): valgrind cachegrind
#                         --smc-check=all on spike-b `bench run A` and `C` (n = SB_CG_N), and on
#                         `sp-dispatch run sibling loop.ep9d` and `sp-dispatch ops`
#
# Verdict: void if the bench says void (IQR of t_A or t_C > 5%); else provisional if the host
# is not the gate host, a load guard failed or did not run, tau is older than
# SB_TAU_MAX_AGE_MIN, or the build or round count is not the gate's; else the bench verdict
# (pass, fail; "marginal" stays as a label). Exit 0 when the run is complete, whatever the
# verdict; 1 if the run is invalid (a wrong `drive` result, a missing input); 2 on a usage
# error; 3 "host busy" (the load guard before failed after SB_WAIT_MIN, without SB_ALLOW_BUSY).
#
# Env:
#   SB_BIN               spike-b binary (default: $CARGO_TARGET_DIR or spikes/spike-b/target,
#                        then release/spike-b[.exe])
#   SB_TAU_JSON          SP-Dispatch bench.json (default spikes/sp-dispatch/target/gate/bench.json)
#   SB_TAU_MAX_AGE_MIN   120: an older bench.json makes the run provisional
#   SB_ROUNDS=15  SB_MARGINAL_ROUNDS=31  SB_N=100000000
#   SB_HOST_FILE         tools/bench/hosts/m5max-macos.toml (the gate host, protocol §7)
#   SB_GUARD=1           0 skips the load guard (the run is then provisional)
#   SB_SETTLE_S=60  SB_WAIT_MIN=30  SB_AFTER_S=120   load guard timing (protocol §7)
#   SB_ALLOW_BUSY=0      1: a failed guard before does not stop the run; it is provisional
#   SB_COMMIT            commit text when the tree has no git (the x86 sandbox)
#   SB_SPD_BIN           sp-dispatch binary for cachegrind (spikes/sp-dispatch/target/release/sp-dispatch)
#   SB_CG_N=1000000      n under cachegrind
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
exe=""
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) exe=".exe" ;; esac
bin="${SB_BIN:-${CARGO_TARGET_DIR:-$here/target}/release/spike-b$exe}"
tau_json="${SB_TAU_JSON:-$root/spikes/sp-dispatch/target/gate/bench.json}"
mode="${1:-run}"

die() { echo "bench.sh: $*" >&2; exit "${2:-1}"; }
load() { uptime 2>/dev/null || echo "n/a (no uptime command)"; }
commit() {
  if [ -n "${SB_COMMIT:-}" ]; then echo "$SB_COMMIT"; return; fi
  local c
  c="$(git -C "$root" rev-parse HEAD 2>/dev/null)" || { echo "unknown (no git; set SB_COMMIT)"; return; }
  if git -C "$root" diff --quiet HEAD -- 2>/dev/null; then echo "$c"; else echo "$c (+ uncommitted changes)"; fi
}
[ -x "$bin" ] || die "no spike-b binary at $bin: build it first (cargo build --locked --release --manifest-path spikes/spike-b/Cargo.toml); bench.sh builds nothing"

# ---- cachegrind (protocol §8) -------------------------------------------------------------------
cachegrind() {
  command -v valgrind >/dev/null || die "cachegrind: valgrind is not on PATH"
  local spd="${SB_SPD_BIN:-$root/spikes/sp-dispatch/target/release/sp-dispatch$exe}"
  [ -x "$spd" ] || die "cachegrind: no sp-dispatch binary at $spd (cargo build --release in spikes/sp-dispatch)"
  local n="${SB_CG_N:-1000000}" tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  # ir <name> <cmd...>: the "I refs" of one cachegrind run; the command's stdout in $tmp/<name>.out.
  ir() {
    local name=$1
    shift
    valgrind --tool=cachegrind --smc-check=all --cache-sim=no --cachegrind-out-file=/dev/null \
      "$@" >"$tmp/$name.out" 2>"$tmp/$name.err" || { cat "$tmp/$name.err" >&2; return 1; }
    sed -n 's/^==[0-9]*== I *refs: *//p' "$tmp/$name.err" | tr -d ', '
  }
  echo "== Spike B cachegrind ratio (protocol §8; a report, never a gate)"
  echo "host: $(uname -sm); commit: $(commit)"
  echo "valgrind: $(valgrind --version); n = $n"
  local ir_a ir_c ir_loop ir_ops
  ir_a="$(ir A "$bin" bench run A --n "$n")" || die "cachegrind A failed"
  ir_c="$(ir C "$bin" bench run C --n "$n")" || die "cachegrind C failed"
  grep -h '^spike-b-run ' "$tmp/A.out" "$tmp/C.out"
  for c in A C; do grep -q "result=$n\$" "$tmp/$c.out" || die "cachegrind: run $c did not return $n"; done
  "$spd" dump loop default "$tmp/loop.ep9d" || die "sp-dispatch dump failed"
  ir_loop="$(ir loop "$spd" run sibling "$tmp/loop.ep9d")" || die "cachegrind loop failed"
  echo "sp-dispatch run sibling loop.ep9d: $(cat "$tmp/loop.out")"
  ir_ops="$(ir ops "$spd" ops)" || die "cachegrind ops failed"
  # The loop kernel at its default n has 880,000,017 dispatches (protocol §3, scale 1). Take the
  # number from bench.json when one is here.
  local disp=880000017 src="protocol §3 (loop kernel, default n)"
  if [ -f "$tau_json" ]; then
    local d
    d="$(python3 -I -c 'import json,sys
d=json.load(open(sys.argv[1]))
print(next(k["dispatches"] for k in d["kernels"] if k["name"]=="loop") if d.get("scale")==1 else "")' "$tau_json" 2>/dev/null)"
    [ -n "$d" ] && { disp="$d"; src="$tau_json"; }
  fi
  echo "dispatches of the loop kernel: $disp ($src)"
  "$bin" bench ratio --ir-a "$ir_a" --ir-c "$ir_c" --ir-loop "$ir_loop" --ir-ops "$ir_ops" \
    --dispatches "$disp" --n "$n"
}

case "$mode" in
  run) ;;
  cachegrind) cachegrind; exit $? ;;
  *) die "usage: bench.sh [run | cachegrind]" 2 ;;
esac

# ---- 1. tau --------------------------------------------------------------------------------------
[ -f "$tau_json" ] || die "no $tau_json: run 'bash spikes/sp-dispatch/gate.sh' (SP_SPEED=gate on the gate host) first, in the same session (protocol §3)"
# Prints key=value lines, or "error=<why>".
tau_py='import json, os, sys, time
p = sys.argv[1]
try:
    d = json.load(open(p))
except Exception as e:
    print("error=cannot read %s: %s" % (p, e)); sys.exit(0)
errs = []
if d.get("release") != "sibling": errs.append("release is %r, not sibling (H15)" % d.get("release"))
if d.get("scale") != 1: errs.append("scale is %r, not 1" % d.get("scale"))
if d.get("results_equal") is not True: errs.append("results_equal is not true")
loop = [k for k in d.get("kernels", []) if k.get("name") == "loop"]
if len(loop) != 1: errs.append("no kernel named loop")
if errs:
    print("error=" + "; ".join(errs)); sys.exit(0)
k = loop[0]
print("tau=%.9f" % (k["median_ns"]["sibling"] / k["dispatches"]))
print("dispatches=%d" % k["dispatches"])
print("samples=" + ",".join(repr(x) for x in k["samples_ns"]["sibling"]))
print("cpu=%s" % d.get("cpu"))
print("rounds=%s" % d.get("rounds"))
print("age_min=%.1f" % ((time.time() - os.path.getmtime(p)) / 60))
print("uptime_before=%s" % d.get("uptime_before"))
print("uptime_after=%s" % d.get("uptime_after"))'
tau_js='const fs = require("fs"); const p = process.argv[1]; let d;
try { d = JSON.parse(fs.readFileSync(p, "utf8")); } catch (e) { console.log(`error=cannot read ${p}: ${e.message}`); process.exit(0); }
const errs = [];
if (d.release !== "sibling") errs.push(`release is ${d.release}, not sibling (H15)`);
if (d.scale !== 1) errs.push(`scale is ${d.scale}, not 1`);
if (d.results_equal !== true) errs.push("results_equal is not true");
const loop = (d.kernels || []).filter((k) => k.name === "loop");
if (loop.length !== 1) errs.push("no kernel named loop");
if (errs.length) { console.log("error=" + errs.join("; ")); process.exit(0); }
const k = loop[0];
console.log(`tau=${(k.median_ns.sibling / k.dispatches).toFixed(9)}`);
console.log(`dispatches=${k.dispatches}`);
console.log(`samples=${k.samples_ns.sibling.join(",")}`);
console.log(`cpu=${d.cpu}`); console.log(`rounds=${d.rounds}`);
console.log(`age_min=${((Date.now() - fs.statSync(p).mtimeMs) / 60000).toFixed(1)}`);
console.log(`uptime_before=${d.uptime_before}`); console.log(`uptime_after=${d.uptime_after}`);'
if command -v python3 >/dev/null; then
  tau_out="$(python3 -I -c "$tau_py" "$tau_json")"
elif command -v node >/dev/null; then
  tau_out="$(node -e "$tau_js" "$tau_json")"
else
  die "neither python3 nor node is on PATH: cannot read $tau_json"
fi
field() { sed -n "s/^$1=//p" <<<"$tau_out" | head -n 1; }
[ -z "$(field error)" ] || die "$tau_json: $(field error)"
tau="$(field tau)"
[ -n "$tau" ] || die "$tau_json: no tau"
tau_age="$(field age_min)"
reasons=()

# ---- 2. host check and load guard before -------------------------------------------------------
host_file="${SB_HOST_FILE:-$root/tools/bench/hosts/m5max-macos.toml}"
settle="${SB_SETTLE_S:-60}" wait_min="${SB_WAIT_MIN:-30}" after_s="${SB_AFTER_S:-120}"
guard_js='const [mode, lib, hostFile, a, b] = process.argv.slice(1);
const { pathToFileURL } = require("url");
import(pathToFileURL(lib).href).then(async (h) => {
  const host = h.readHostFile(hostFile);
  const sleep = (s) => new Promise((r) => setTimeout(r, s * 1000));
  if (mode === "host") {
    const cpu = h.cpuModel(), plat = `${process.platform}-${process.arch}`;
    const ok = host.gate_host === true && cpu === host.cpu && plat === host.platform;
    console.log(`${ok ? "GATE-HOST" : "NOT-GATE-HOST"} ${host.id}: cpu "${cpu}" (host file "${host.cpu}"), platform ${plat} (host file ${host.platform}), gate_host ${host.gate_host}`);
    return;
  }
  const c = mode === "before"
    ? await h.guardBefore({ idleBaseline: host.idle_baseline_load1, settleS: Number(a), waitS: Number(b) * 60 }, { sleep })
    : await h.guardAfter({ idleBaseline: host.idle_baseline_load1, afterS: Number(a) }, { sleep });
  const waited = mode === "before" ? `, settled ${c.settle_s} s, waited ${c.waited_s} s` : `, after ${c.after_s} s`;
  console.log(`${c.pass ? "PASS" : "FAIL"} load ${c.load1}, limit ${c.limit}${waited}${c.why ? ` (${c.why})` : ""}`);
}).catch((e) => { console.log(`ERROR ${e.message}`); });'
have_node=0
command -v node >/dev/null && have_node=1
guard() { # guard <host|before|after> [args]: one line from host.mjs
  node -e "$guard_js" "$1" "$root/tools/bench/lib/host.mjs" "$host_file" "${2:-}" "${3:-}"
}

echo "== Spike B D21 bench.sh (docs/measurements/2026-10-22-spike-b-protocol.md)"
echo "binary: $bin ($("$bin"))"
echo "commit: $(commit)"
echo "uptime: $(load)"
echo "tau: $tau ns = median_ns.sibling / dispatches of kernel loop ($(field dispatches) dispatches)"
echo "tau source: $tau_json (cpu \"$(field cpu)\", $(field rounds) rounds, written $tau_age min ago)"
echo "tau source uptime before: $(field uptime_before)"
echo "tau source uptime after:  $(field uptime_after)"
if awk -v a="$tau_age" -v m="${SB_TAU_MAX_AGE_MIN:-120}" 'BEGIN { exit !(a > m) }'; then
  reasons+=("tau is $tau_age min old (> ${SB_TAU_MAX_AGE_MIN:-120}; protocol §3 same session)")
fi
if [ "$have_node" = 1 ]; then
  host_line="$(guard host)"
  echo "gate host: $host_line"
  case "$host_line" in GATE-HOST*) ;; *) reasons+=("not the gate host ($host_line)") ;; esac
else
  echo "gate host: not checked (no node)"
  reasons+=("no node: host check and load guard not run")
fi
guard_before="not run"
if [ "${SB_GUARD:-1}" = 1 ] && [ "$have_node" = 1 ]; then
  echo "load guard before: $settle s with no benchmark process, then load <= idle + 0.5 (wait up to $wait_min min)"
  guard_before="$(guard before "$settle" "$wait_min")"
  echo "load guard before: $guard_before"
  case "$guard_before" in
    PASS*) ;;
    *)
      if [ "${SB_ALLOW_BUSY:-0}" != 1 ]; then
        echo "host busy: the load guard before failed (protocol §7); set SB_ALLOW_BUSY=1 for a provisional run"
        exit 3
      fi
      reasons+=("load guard before: $guard_before") ;;
  esac
elif [ "$have_node" = 1 ]; then
  echo "load guard before: not run (SB_GUARD=0)"
  reasons+=("load guard not run (SB_GUARD=0)")
fi

# ---- 3. the bench --------------------------------------------------------------------------------
out="$(mktemp)"
trap 'rm -f "$out"' EXIT
"$bin" bench --tau "$tau" --tau-samples "$(field samples)" --dispatches "$(field dispatches)" \
  --tau-source "$tau_json" --rounds "${SB_ROUNDS:-15}" --marginal-rounds "${SB_MARGINAL_ROUNDS:-31}" \
  --n "${SB_N:-100000000}" --commit "$(commit)" | tee "$out"
st="${PIPESTATUS[0]}"
[ "$st" = 0 ] || die "spike-b bench exited $st: the run is invalid (protocol §2: a wrong result fails the run, not the gate)"
res="$(grep -m1 '^SPIKE-B-RESULT ' "$out")" || die "no SPIKE-B-RESULT line"

# ---- 4. load guard after, verdict ------------------------------------------------------------------
guard_after="not run"
if [ "${SB_GUARD:-1}" = 1 ] && [ "$have_node" = 1 ]; then
  echo
  echo "load guard after: wait $after_s s, then load <= idle + 0.5"
  guard_after="$(guard after "$after_s")"
  echo "load guard after: $guard_after"
  case "$guard_after" in PASS*) ;; *) reasons+=("load guard after: $guard_after") ;; esac
fi
raw="$(awk '{ print $2 }' <<<"$res")"
labels="$(sed -n 's/.* labels=//p' <<<"$res")"
[ "$labels" = "-" ] && labels=""
# "marginal" is a label of the verdict, not a reason for "provisional"; the others are (a debug
# build, fewer than 15 kept rounds).
other="$(tr ';' '\n' <<<"$labels" | sed 's/^ *//' | grep -v '^marginal$' | grep -v '^$' | paste -sd ';' -)"
if [ -n "$other" ]; then reasons+=("bench labels: $other"); fi
case "$raw" in pass | fail | void) ;; *) die "unknown bench verdict '$raw'" ;; esac
if [ "$raw" = void ]; then
  verdict=void
elif [ "${#reasons[@]}" -gt 0 ]; then
  verdict=provisional
else
  verdict="$raw"
fi
echo
echo "== Spike B D21 result"
echo "load before: $guard_before"
echo "load after:  $guard_after"
echo "uptime: $(load)"
echo "bench: $res"
echo "VERDICT: $verdict (bench verdict: $raw$(case "$labels" in *marginal*) echo ", marginal" ;; esac))"
for r in ${reasons[@]+"${reasons[@]}"}; do echo "  label: $r"; done
exit 0
