#!/usr/bin/env bash
# Runs Fuzzilli (the ep9 profile) on `ep9 --reprl` for <minutes> and writes the coverage lines
# to a log. Mutation run: only inside the infra/fuzz image on Cloudflare, never on the Mac
# (plan v9 rule 23, V9-A6).
#
#   infra/fuzz/run.sh <minutes> [<out dir>]        out dir default /work/fuzz-out (made fresh)
#
# Environment: EP9 (default /opt/ep9/bin/ep9), FUZZILLI (default /opt/fuzzilli/bin/FuzzilliCli),
# JOBS (Fuzzilli --jobs, default nproc), FUZZ_TAG (Fuzzilli --tag, default ep9-<hostname>).
#
# <out dir>/fuzzilli.log   everything Fuzzilli prints (it holds program text: keep it on the box)
# <out dir>/storage/       Fuzzilli's storage: crashes/, corpus/, settings.json
# <out dir>/coverage.log   one line per stats print (every 60 s):
#                          <uptime>  coverage <c>%  crashes <n>  execs <n>
# Prints the coverage lines, then the crash count and the SHA-256 of each crash file (no
# program text, plan v9 rule 14).
#
# Exit 0: coverage grew from the first coverage line to the last. Exit 1: it did not, or fewer
# than 2 lines. Exit 2: bad usage, or Fuzzilli wrote no coverage line (start-up tests failed).
set -uo pipefail

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

rm -rf "$out"
mkdir -p "$out"
echo "run.sh: $minutes min, $jobs jobs, tag $tag, ep9 $(sha256sum "$ep9" | cut -c1-16)"
# --maxRuntime stops Fuzzilli; timeout is only a guard (start-up tests, shutdown).
timeout --signal=INT --kill-after=60 "$((minutes * 60 + 300))" \
  "$fuzzilli" --profile=ep9 --jobs="$jobs" --maxRuntime="${minutes}m" --tag="$tag" \
  --storagePath="$out/storage" "$ep9" > "$out/fuzzilli.log" 2>&1
rc=$?
echo "run.sh: FuzzilliCli exit $rc"

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
