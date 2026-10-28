#!/usr/bin/env bash
# Runs Fuzzilli (the ep9 profile) on `ep9 --reprl` for <minutes> and writes the coverage lines
# to a log. Mutation run: only inside the infra/fuzz image on Cloudflare, never on the Mac
# (plan v9 rule 23, V9-A6).
#
#   infra/fuzz/run.sh <minutes> [<out dir>]        out dir default /work/fuzz-out (made fresh)
#
# Environment: EP9 (default /opt/ep9/bin/ep9; the jitless build is /opt/ep9/bin/ep9-jitless,
# $EP9_JITLESS in the image), FUZZILLI (default /opt/fuzzilli/bin/FuzzilliCli), JOBS (Fuzzilli
# --jobs, default nproc), FUZZ_TAG (Fuzzilli --tag, default ep9-<hostname>). The ep9 profile
# forwards EP9_REPRL_HEAP_MB, EP9_REPRL_STACK_MB, EP9_REPRL_GC_STRESS and EP9_REPRL_PRESET
# (jit | jitless) from this environment to the REPRL child.
# The ep9-fuzz/v1 record (fuzz/scripts/record.sh; plan v10 §2.7):
#   FUZZ_BOX      box name in the campaign id (default the hostname)
#   FUZZ_SESSION  session name in the campaign id (default the UTC start time, HHMMSS)
#   COUNTED       true | false (default false); true only for a run pre-registered by a
#                 decisions-log row (plan rule 41)
#   EP9_SOURCE    the source file of the image (default /opt/ep9/SOURCE: commit=, dirty=)
# Target: `reprl-jitless` when line 2 of `$EP9 --version` says "preset jitless" (the jitless
# build), else `reprl`.
# Campaign id: fz-<UTC date YYYYMMDD>-<box>-<session>, one per box session (m1-exit counts each
# id once). cpu_hours = the measured FuzzilliCli seconds x JOBS / 3600. Crash ids: the triage
# group ids (infra/fuzz/triage.sh --ids). A FuzzilliCli exit other than 0, or no coverage line,
# writes counted false. An image with no source commit writes commit "unknown" and no `dirty`
# field: m1-exit does not count it.
#
# <out dir>/fuzzilli.log   everything Fuzzilli prints (it holds program text: keep it on the box)
# <out dir>/storage/       Fuzzilli's storage: crashes/, corpus/, settings.json
# <out dir>/coverage.log   one line per stats print (every 60 s):
#                          <uptime>  coverage <c>%  crashes <n>  execs <n>
# <out dir>/record.json    the ep9-fuzz/v1 record of this session (names, numbers, hashes)
# Prints the coverage lines, then the crash count and the SHA-256 of each crash file, then the
# record (no program text, plan v9 rule 14).
#
# Exit 0: FuzzilliCli ended normally and coverage grew from the first coverage line to the last.
# FuzzilliCli ends normally with exit 0 when --maxRuntime is reached (ShutdownReason.finished);
# a fatal error is exit 255 (-1, "Aborting execution after a fatal error."), and the watchdog
# below makes 124 (SIGINT at the limit) or 137 (SIGKILL 60 s later). Exit 1: a nonzero FuzzilliCli
# exit, or coverage did not grow, or fewer than 2 lines. Exit 2: bad usage, or Fuzzilli wrote no
# coverage line (start-up tests failed). A failure prints the exit code, its cause, the coverage
# lines and the crash hashes; the program text stays in fuzzilli.log on the box. A record that
# cannot be written is exit 2.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
record_sh="$here/../../fuzz/scripts/record.sh"

minutes="${1:-}"
out="${2:-/work/fuzz-out}"
case "$minutes" in
  '' | *[!0-9]* | 0) echo "usage: infra/fuzz/run.sh <minutes> [<out dir>]" >&2; exit 2 ;;
esac
ep9="${EP9:-/opt/ep9/bin/ep9}"
fuzzilli="${FUZZILLI:-/opt/fuzzilli/bin/FuzzilliCli}"
jobs="${JOBS:-$(nproc)}"
tag="${FUZZ_TAG:-ep9-$(hostname)}"
for f in "$ep9" "$fuzzilli"; do
  [ -x "$f" ] || { echo "run.sh: $f is not an executable" >&2; exit 2; }
done
[ -f "$record_sh" ] || { echo "run.sh: $record_sh is missing" >&2; exit 2; }
case "$jobs" in '' | *[!0-9]* | 0) echo "run.sh: JOBS must be a whole number >= 1" >&2; exit 2 ;; esac
counted="${COUNTED:-false}"
case "$counted" in true | false) ;; *) echo "run.sh: COUNTED must be true or false" >&2; exit 2 ;; esac
# Campaign id parts: letters, digits, '.' and '-' only (any other character becomes '-').
box="$(printf '%s' "${FUZZ_BOX:-$(hostname)}" | tr -c 'A-Za-z0-9.-' '-')"
session="$(printf '%s' "${FUZZ_SESSION:-$(date -u +%H%M%S)}" | tr -c 'A-Za-z0-9.-' '-')"
campaign="fz-$(date -u +%Y%m%d)-$box-$session"
# The source state that the image was built from (infra/fuzz/Dockerfile writes it).
source_file="${EP9_SOURCE:-/opt/ep9/SOURCE}"
src_commit=unknown src_dirty=unknown
if [ -f "$source_file" ]; then
  src_commit="$(sed -n 's/^commit=//p' "$source_file" | head -1)"
  src_dirty="$(sed -n 's/^dirty=//p' "$source_file" | head -1)"
fi
case "$src_commit" in *[!0-9a-f]* | '') src_commit=unknown ;; esac
[ "${#src_commit}" -eq 40 ] || src_commit=unknown
case "$src_dirty" in true | false) ;; *) src_dirty=unknown ;; esac
[ "$src_commit" = unknown ] && src_dirty=unknown
# The build line of the binary ("features <list>; preset <name>"; ep9_vm::build_info).
build="$("$ep9" --version 2>/dev/null | sed -n 2p)"
case "$build" in
  *"; preset jitless") target=reprl-jitless ;;
  *) target=reprl ;;
esac

rm -rf "$out"
mkdir -p "$out"
echo "run.sh: $minutes min, $jobs jobs, tag $tag, ep9 $(sha256sum "$ep9" | cut -c1-16)"
echo "run.sh: target $target, build ${build:-unknown}, preset setting ${EP9_REPRL_PRESET:-default}"
echo "run.sh: campaign $campaign, commit $src_commit, dirty $src_dirty, counted $counted"
start=$(date +%s)
# --maxRuntime stops Fuzzilli; timeout is only a guard (start-up tests, shutdown).
timeout --signal=INT --kill-after=60 "$((minutes * 60 + 300))" \
  "$fuzzilli" --profile=ep9 --jobs="$jobs" --maxRuntime="${minutes}m" --tag="$tag" \
  --storagePath="$out/storage" "$ep9" > "$out/fuzzilli.log" 2>&1
rc=$?
secs=$(($(date +%s) - start))
case "$rc" in
  0) why="finished: --maxRuntime reached" ;;
  124) why="the watchdog sent SIGINT after $((minutes * 60 + 300)) s: --maxRuntime did not end it" ;;
  137) why="killed by SIGKILL (the watchdog 60 s after its SIGINT, or the kernel)" ;;
  255) why="a Fuzzilli fatal error" ;;
  *) why="an abnormal end" ;;
esac
echo "run.sh: FuzzilliCli exit $rc ($why)"

awk '/^Uptime:/ { u = $2 " " $3 " " $4 " " $5 }
     /^Coverage:/ { c = $2 }
     /^Crashes Found:/ { k = $3 }
     /^Total Execs:/ { printf "%s  coverage %s  crashes %s  execs %s\n", u, c, k, $3 }' \
  "$out/fuzzilli.log" > "$out/coverage.log"
cat "$out/coverage.log"

crashes=()
if [ -d "$out/storage/crashes" ]; then
  while IFS= read -r f; do crashes+=("$f"); done \
    < <(find "$out/storage/crashes" -maxdepth 1 -name '*.js' | sort)
fi
echo "run.sh: ${#crashes[@]} unique crash file(s)"
for f in ${crashes[@]+"${crashes[@]}"}; do
  echo "  $(sha256sum "$f" | cut -d' ' -f1)  $(basename "$f")"
done

n="$(wc -l < "$out/coverage.log" | tr -d ' ')"

# The ep9-fuzz/v1 record: crash ids are the triage group ids (never a program text).
rec=(--tool fuzzilli --campaign "$campaign" --target "$target" --commit "$src_commit"
  --dirty "$src_dirty" --counted "$counted" --seconds "$secs" --workers "$jobs" --binary "$ep9")
if [ "${#crashes[@]}" -gt 0 ]; then
  ids="$(bash "$here/triage.sh" --ids ${crashes[@]+"${crashes[@]}"})" ||
    { echo "run.sh: FAIL: triage.sh --ids failed; no record written"; exit 2; }
  for id in $ids; do rec+=(--crash-id "$id"); done
fi
if [ "$rc" -ne 0 ]; then
  rec+=(--uncounted "FuzzilliCli exit $rc")
elif [ "$n" -eq 0 ]; then
  rec+=(--uncounted "no coverage line")
fi
bash "$record_sh" "${rec[@]}" --out "$out/record.json" ||
  { echo "run.sh: FAIL: record.sh failed; no record written"; exit 2; }
echo "run.sh: record $out/record.json"
cat "$out/record.json"

if [ "$rc" -ne 0 ]; then
  fatal=no
  grep -q 'Aborting execution after a fatal error' "$out/fuzzilli.log" && fatal=yes
  echo "run.sh: FAIL: FuzzilliCli exit $rc ($why); $n coverage line(s), fatal-error line: $fatal (see $out/fuzzilli.log)"
  [ "$n" -eq 0 ] && exit 2
  exit 1
fi
if [ "$n" -eq 0 ]; then
  echo "run.sh: FAIL: no coverage line (see $out/fuzzilli.log)"
  exit 2
fi
first="$(head -1 "$out/coverage.log" | awk '{ print $6 }' | tr -d '%')"
last="$(tail -1 "$out/coverage.log" | awk '{ print $6 }' | tr -d '%')"
if [ "$n" -ge 2 ] && awk -v a="$first" -v b="$last" 'BEGIN { exit !(b > a) }'; then
  echo "run.sh: PASS: coverage grew from $first% to $last% ($n lines)"
  exit 0
fi
echo "run.sh: FAIL: coverage did not grow ($first% to $last%, $n lines)"
exit 1
