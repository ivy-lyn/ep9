#!/usr/bin/env bash
# Runs one front-end fuzz target and prints ONLY the crash count, the SHA-256 of each crash
# file and libFuzzer's numeric stats (plan v9 rule 14 and V9-A17: the repo is public, and a
# libFuzzer or cargo-fuzz crash report prints the input bytes). Everything the fuzzer prints
# goes to $FUZZ_LOG_DIR/<target>.log, which stays on the machine or inside the CI job.
#
#   fuzz/scripts/run.sh build [<target>]                     cargo fuzz build (its output is
#                                                            compiler text, printed as is)
#   fuzz/scripts/run.sh <target> replay  <corpus dir>...     each input once, no mutation
#                                                            (-runs=0; the only mode for the Mac)
#   fuzz/scripts/run.sh <target> <seconds> <corpus dir>...   mutation run (CI and campaigns)
#   fuzz/scripts/run.sh crashes <summary file>               the crash ids of a run cut off
#                                                            before its summary (below)
#
# The first corpus dir is the writable one; the others are read only (seeds). Extra libFuzzer
# flags go after "--". Environment:
#   FUZZ_SANITIZER  address (default) or none     FUZZ_FEATURES  cargo features (self-test)
#   FUZZ_FORK       n > 0: -fork=n, crashes counted, the run goes on (campaigns)
#   FUZZ_LOG_DIR    default fuzz/logs              FUZZ_SUMMARY   write a JSON summary here
#   FUZZ_TIMEOUT    seconds per input (default 30) FUZZ_MAX_LEN   -max_len (default 4096)
#
# The summary (numbers, names and hashes only) is the input of fuzz/scripts/record.sh --run:
# "seconds" is the measured wall time of the run, "workers" the -fork workers (1 without fork),
# "binary_sha256" the SHA-256 of the target binary that ran (null when it is not found).
# When the run starts, after it empties its artifact dir, run.sh writes the start marker
# <summary file>.started (that dir's path). `run.sh crashes <summary file>` prints the SHA-256
# of each crash file in the dir its marker names, one per line, and nothing when there is no
# marker (no run started with that summary file, so an older run's files, such as the
# self-test's fixture crash, are never read). fuzz.yml's Summary step uses it when the
# campaign step ended with no summary.
#
# Exit 0: no crash file and libFuzzer exit 0. Exit 1: a fixed text and the hashes only.
# A crash file is every artifact file except libFuzzer's `slow-unit-*` (an input that took at
# least -report_slow_units seconds, default 10, but less than -timeout): those are listed with
# their hashes but do not fail the run (a timeout is a `timeout-*` file and does).
set -uo pipefail

fuzz_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
caller_dir="$PWD"
cd "$fuzz_dir" || exit 2
san="${FUZZ_SANITIZER:-address}"
feat=()
if [ -n "${FUZZ_FEATURES:-}" ]; then
  feat=(--features "$FUZZ_FEATURES")
fi

if [ "${1:-}" = build ]; then
  shift
  exec cargo fuzz build -s "$san" ${feat[@]+"${feat[@]}"} "$@"
fi

usage() {
  echo "usage: $0 build [<target>] | $0 crashes <summary file> | $0 <target> replay|<seconds> <corpus dir>... [-- <libFuzzer flags>]" >&2
  exit 2
}
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}
# A crash file is every artifact file except libFuzzer's slow-unit-* (see the header).
is_crash() {
  case "${1##*/}" in slow-unit-*) return 1 ;; *) return 0 ;; esac
}
# <summary file> -> its path (relative to the caller's directory).
summary_path() {
  case "$1" in /*) printf '%s' "$1" ;; *) printf '%s' "$caller_dir/$1" ;; esac
}

if [ "${1:-}" = crashes ]; then
  [ $# -eq 2 ] || usage
  marker="$(summary_path "$2").started"
  [ -f "$marker" ] || exit 0
  dir="$(cat "$marker")"
  [ -d "$dir" ] || { echo "run.sh crashes: the started run's artifact dir is gone" >&2; exit 2; }
  while IFS= read -r f; do
    if is_crash "$f"; then sha256 "$f" || exit 2; fi
  done < <(find "$dir" -type f | LC_ALL=C sort)
  exit 0
fi
[ $# -ge 3 ] || usage
target="$1" mode="$2"
shift 2
corpus=()
while [ $# -gt 0 ] && [ "$1" != -- ]; do
  # Relative to the caller's directory (this script runs in fuzz/).
  case "$1" in
    /*) corpus+=("$1") ;;
    *) corpus+=("$caller_dir/$1") ;;
  esac
  shift
done
[ "${1:-}" = -- ] && shift
[ ${#corpus[@]} -ge 1 ] || usage

log_dir="${FUZZ_LOG_DIR:-$fuzz_dir/logs}"
case "$log_dir" in /*) ;; *) log_dir="$caller_dir/$log_dir" ;; esac
mkdir -p "$log_dir"
log="$log_dir/$target.log"
art="artifacts/$target"
rm -rf "$art"
mkdir -p "$art"
summary=""
if [ -n "${FUZZ_SUMMARY:-}" ]; then
  summary="$(summary_path "$FUZZ_SUMMARY")"
  rm -f "$summary" # an older run's summary is not this run's
  printf '%s\n' "$fuzz_dir/$art" >"$summary.started"
fi
for d in "${corpus[@]}"; do
  mkdir -p "$d"
done

args=(
  "-artifact_prefix=$art/"
  "-dict=dict/js.dict"
  "-timeout=${FUZZ_TIMEOUT:-30}"
  "-max_len=${FUZZ_MAX_LEN:-4096}"
  "-rss_limit_mb=4096"
  "-print_final_stats=1"
)
case "$mode" in
  replay) args+=("-runs=0") ;;
  '' | *[!0-9]*) usage ;;
  *) args+=("-max_total_time=$mode") ;;
esac
fork="${FUZZ_FORK:-0}"
if [ "$fork" -gt 0 ] 2>/dev/null; then
  args+=("-fork=$fork" "-ignore_crashes=1" "-ignore_timeouts=1" "-ignore_ooms=1")
fi

start=$(date +%s)
cargo fuzz run -s "$san" ${feat[@]+"${feat[@]}"} "$target" "${corpus[@]}" -- "${args[@]}" "$@" >"$log" 2>&1
rc=$?
secs=$(($(date +%s) - start))

files=()
slow=()
while IFS= read -r f; do
  if is_crash "$f"; then files+=("$f"); else slow+=("$f"); fi
done < <(find "$art" -type f | LC_ALL=C sort)
n=${#files[@]}
echo "fuzz $target ($mode, ${secs}s): libFuzzer exit $rc, $n crash file(s), ${#slow[@]} slow unit(s)"
hashes=()
for f in ${files[@]+"${files[@]}"}; do
  base="${f##*/}"
  kind="${base%%-*}"
  h="$(sha256 "$f")"
  hashes+=("$h")
  echo "  $kind sha256:$h"
done
slow_hashes=()
for f in ${slow[@]+"${slow[@]}"}; do
  h="$(sha256 "$f")"
  slow_hashes+=("$h")
  echo "  slow-unit sha256:$h (not a failure)"
done
# Numbers only: no line of the log is printed unless it is "stat::<name>: <integer>" or
# libFuzzer's random seed ("INFO: Seed: <integer>"; with the commit and the seed corpus it
# replays a single-process run on a private host).
grep -E '^(stat::[a-z_]+|INFO: Seed): [0-9]+$' "$log" | sed 's/^/  /'

if [ -n "$summary" ]; then
  execs="$(grep -E '^stat::number_of_executed_units: [0-9]+$' "$log" | tail -1 | sed 's/.*: //')"
  workers=1
  [ "$fork" -gt 0 ] 2>/dev/null && workers="$fork"
  # cargo fuzz builds <target> for the host triple in release mode (cargo-fuzz 0.12 defaults).
  host="$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')"
  bin_sha=null
  if [ -n "$host" ] && [ -f "target/$host/release/$target" ]; then
    bin_sha="\"$(sha256 "target/$host/release/$target")\""
  fi
  {
    printf '{"tool":"cargo-fuzz","target":"%s","mode":"%s","seconds":%s,"fork":%s,"workers":%s,' \
      "$target" "$mode" "$secs" "$fork" "$workers"
    printf '"binary_sha256":%s,' "$bin_sha"
    printf '"libfuzzer_exit":%s,"executed_units":%s,"crashes":%s,"crash_sha256":[' \
      "$rc" "${execs:-null}" "$n"
    sep=""
    for h in ${hashes[@]+"${hashes[@]}"}; do
      printf '%s"%s"' "$sep" "$h"
      sep=","
    done
    printf '],"slow_units":%s,"slow_sha256":[' "${#slow[@]}"
    sep=""
    for h in ${slow_hashes[@]+"${slow_hashes[@]}"}; do
      printf '%s"%s"' "$sep" "$h"
      sep=","
    done
    printf ']}\n'
  } >"$summary"
fi

if [ "$n" -gt 0 ]; then
  echo "fuzz $target: FAILED: $n crash file(s). The inputs and the log stay inside this job."
  exit 1
fi
if [ "$rc" -ne 0 ]; then
  echo "fuzz $target: FAILED: libFuzzer exit $rc with no crash file. The log stays inside this job."
  exit 1
fi
exit 0
