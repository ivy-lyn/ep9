#!/usr/bin/env bash
# The verdict logic of bench.sh (decision M2-P3-SPIKEB-VERDICT), without the bench: SB_BIN is a
# fake spike-b that prints a given SPIKE-B-RESULT line, SB_SPD_BIN a fake sp-dispatch that
# prints a given CPU, SB_TAU_JSON a written bench.json, and SB_HOST_FILE a host file of this
# machine with a load guard that always passes (no wait). So each case changes one input and
# checks the VERDICT line and the labels. gate.sh structural runs it.
#
#   bench-verdict-test.sh [bench.sh]     (default: the bench.sh next to this file)
#
# Needs node (the host check and load guard of bench.sh) and python3 or node (bench.json).
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bench="${1:-$here/bench.sh}"
root="$(cd "$here/../.." && pwd)"
command -v node >/dev/null || { echo "bench-verdict-test.sh: needs node"; exit 1; }
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# A host file of this machine (cpu and platform as host.mjs reads them), gate host, with an
# idle baseline no load reaches.
# shellcheck disable=SC2016 # JavaScript, not shell
host_js='const { pathToFileURL } = require("url");
import(pathToFileURL(process.argv[1]).href).then((h) => {
  console.log(`${h.cpuModel()}\n${process.platform}-${process.arch}`);
});'
{ read -r host_cpu; read -r host_platform; } < <(node -e "$host_js" "$root/tools/bench/lib/host.mjs")
cat >"$tmp/host.toml" <<EOF
id = "verdict-test"
cpu = "$host_cpu"
cores = 1
platform = "$host_platform"
os = "test"
gate_host = true
idle_baseline_load1 = 1000000
EOF

# fake spike-b: no arguments -> one line; `bench ...` -> $FAKE_RESULT.
cat >"$tmp/spike-b" <<'EOF'
#!/usr/bin/env bash
if [ "$#" = 0 ]; then echo "fake spike-b"; exit 0; fi
echo "SPIKE-B-RESULT $FAKE_RESULT"
EOF
# fake sp-dispatch: `cpu` -> $FAKE_CPU.
cat >"$tmp/sp-dispatch" <<'EOF'
#!/usr/bin/env bash
[ "${1:-}" = cpu ] && echo "$FAKE_CPU"
EOF
chmod +x "$tmp/spike-b" "$tmp/sp-dispatch"

# tau_json <rounds> <cpu>: a bench.json that passes bench.sh's release/scale/results checks.
tau_json() {
  cat >"$tmp/bench.json" <<EOF
{"release":"sibling","scale":1,"results_equal":true,"rounds":$1,"cpu":"$2",
 "uptime_before":"u","uptime_after":"u",
 "kernels":[{"name":"loop","dispatches":880000017,"median_ns":{"sibling":264000000},
 "samples_ns":{"sibling":[263000000,264000000,265000000]}}]}
EOF
}

failed=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; failed=1; }
# check <name> <result line> <want VERDICT text> [<want label text>...]: runs bench.sh with
# the inputs set above and checks its exit code 0, the VERDICT line and each label text.
check() {
  local name="$1" result="$2" want="$3"
  shift 3
  local out st
  out="$(FAKE_RESULT="$result" SB_BIN="$tmp/spike-b" SB_SPD_BIN="${spd_bin:-$tmp/sp-dispatch}" \
    SB_TAU_JSON="$tmp/bench.json" SB_HOST_FILE="$tmp/host.toml" SB_SETTLE_S=0 SB_WAIT_MIN=0 \
    SB_AFTER_S=0 SB_GUARD=1 SB_COMMIT=test bash "$bench" 2>&1)"
  st=$?
  local verdict
  verdict="$(grep '^VERDICT: ' <<<"$out")"
  if [ "$st" != 0 ] || [ "$verdict" != "VERDICT: $want" ]; then
    bad "$name: exit $st, '$verdict', want 'VERDICT: $want'"
    printf '%s\n' "$out" | sed 's/^/     | /'
    return
  fi
  local l
  for l in "$@"; do
    if ! grep '^  label: ' <<<"$out" | grep -qF -- "$l"; then
      bad "$name: no label '$l'"
      grep '^  label: ' <<<"$out" | sed 's/^/     | /'
      return
    fi
  done
  if [ "$#" = 0 ] && grep -q '^  label: ' <<<"$out"; then
    bad "$name: labels on a clean run"
    grep '^  label: ' <<<"$out" | sed 's/^/     | /'
    return
  fi
  ok "$name: $verdict"
}

pass="pass median=10.000 lo=9.000 hi=11.000 rounds=15 k=5 tau=0.300000 labels=-"
cpu="Test CPU 9000"
export FAKE_CPU="$cpu"

tau_json 15 "$cpu"
check "every input is the protocol's" "$pass" "pass (bench verdict: pass)"
check "a fail" "fail median=20.000 lo=19.000 hi=21.000 rounds=15 k=5 tau=0.300000 labels=-" \
  "fail (bench verdict: fail)"
check "a marginal re-run that decided" \
  "pass median=17.900 lo=17.000 hi=18.500 rounds=31 k=5 tau=0.300000 labels=marginal" \
  "pass (bench verdict: pass, marginal)"
check "n is not 10^8 (bench label)" \
  "fail median=20.000 lo=19.000 hi=21.000 rounds=15 k=5 tau=0.300000 labels=n = 1000 (protocol §2: 100000000)" \
  "provisional (bench verdict: fail)" "n = 1000"
check "a marginal result that could not re-run" \
  "marginal median=18.000 lo=17.000 hi=19.000 rounds=15 k=5 tau=0.300000 labels=marginal; marginal result not re-run (--marginal-rounds 15 is not above --rounds 15; protocol §6)" \
  "provisional (bench verdict: marginal)" "marginal result not re-run"

tau_json 5 "$cpu"
check "tau from 5 SP rounds" "$pass" "provisional (bench verdict: pass)" "tau source has 5 rounds"
check "void before provisional" \
  "void median=10.000 lo=9.000 hi=11.000 rounds=15 k=5 tau=0.300000 labels=-" \
  "void (bench verdict: void)" "tau source has 5 rounds"

tau_json 15 "Other CPU 1"
check "tau from another CPU" "$pass" "provisional (bench verdict: pass)" \
  "tau source cpu \"Other CPU 1\" is not this host's \"$cpu\""

tau_json 15 "$cpu"
spd_bin="$tmp/no-such-sp-dispatch"
check "no sp-dispatch binary to check the CPU" "$pass" "provisional (bench verdict: pass)" \
  "tau source cpu not checked"
unset spd_bin

if [ "$failed" = 0 ]; then echo "bench-verdict-test: PASS"; else echo "bench-verdict-test: FAIL"; fi
exit "$failed"
