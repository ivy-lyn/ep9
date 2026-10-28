#!/usr/bin/env bash
# Writes one ep9-fuzz/v1 record: the m1-exit item 5 input (tools/xtask/src/m1_exit.rs, "Fuzz
# records"). One record per cargo-fuzz target run (fuzz.yml) or per Fuzzilli box session
# (infra/fuzz/run.sh). The repo is public (plan rule 14): a record holds names, numbers and
# SHA-256 hashes only, never an input byte, a corpus file or a fuzzer log line.
#
#   fuzz/scripts/record.sh --tool cargo-fuzz|fuzzilli --campaign <id> --target <name>
#       --commit <40 hex>|unknown --dirty true|false|unknown --counted true|false
#       ( --run <run.sh summary JSON>
#       | --seconds <n> --workers <n> [--binary <file>] [--crash-id <sha256>]... [--crash-file <file>]... )
#       [--preset <name>] [--runner <name>] [--uncounted <reason>] [--out <file>]
#
# The record (one JSON line; key order as below):
#   schema "ep9-fuzz/v1"; tool; campaign; target;
#   preset     (optional) the execution preset the target ran (Fuzzilli: the REPRL child's,
#              EP9_REPRL_PRESET or else the build's own; infra/fuzz/run.sh). The target names
#              the binary, so a jit build run as jitless keeps target reprl and has preset jitless
#   commit;
#   dirty      true | false; NO field when unknown, so m1-exit does not count it (CX6-1)
#   cpu_hours  measured seconds x workers / 3600 (4 decimals), never the requested time
#   seconds, workers
#   counted    the pre-registered flag (plan rule 41); written false, with uncounted_reason,
#              when the run ended abnormally (--uncounted; libFuzzer exit != 0 with no crash
#              file; no binary hash)
#   crashes    [{"id": <sha256>, "state": "open"}], sorted, each id once. cargo-fuzz: the crash
#              file SHA-256; Fuzzilli: the SHA-256 of the triage key (infra/fuzz/triage.sh --ids)
#   binary_sha256  the fuzzed binary (null when unknown)
#   runner     (optional) the host image name
#   run        (with --run) run.sh's summary: numbers and hashes only
#
# --run reads seconds, workers, binary_sha256 and crash_sha256 from fuzz/scripts/run.sh's
# FUZZ_SUMMARY file. Every string of the record must match [A-Za-z0-9 ._:,()/-]: a value that
# could carry input text is refused (exit 2) before anything is written.
# Exit 0: the record is written (stdout, or --out). Exit 2: bad usage or a bad value.
set -euo pipefail

die() {
  echo "record.sh: $1" >&2
  exit 2
}
command -v jq >/dev/null 2>&1 || die "jq is required"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

tool="" campaign="" target="" commit="" dirty="" counted="" run="" seconds="" workers=""
binary="" binary_given=no runner="" uncounted="" out="" preset=""
ids=()
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || die "$1 needs a value"
  case "$1" in
    --tool) tool="$2" ;;
    --campaign) campaign="$2" ;;
    --target) target="$2" ;;
    --commit) commit="$2" ;;
    --dirty) dirty="$2" ;;
    --counted) counted="$2" ;;
    --run) run="$2" ;;
    --seconds) seconds="$2" ;;
    --workers) workers="$2" ;;
    --binary) binary="$2" binary_given=yes ;;
    --crash-id) ids+=("$2") ;;
    --crash-file)
      [ -f "$2" ] || die "--crash-file: no such file"
      ids+=("$(sha256 "$2")")
      ;;
    --runner) runner="$2" ;;
    --preset) preset="$2" ;;
    --uncounted) uncounted="$2" ;;
    --out) out="$2" ;;
    *) die "unknown option $1" ;;
  esac
  shift 2
done

name_re='^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$'
hex64_re='^[0-9a-f]{64}$'
reason_re='^[A-Za-z0-9 ._:,()/-]{1,200}$'
case "$tool" in cargo-fuzz | fuzzilli) ;; *) die "--tool must be cargo-fuzz or fuzzilli" ;; esac
[[ "$campaign" =~ $name_re ]] || die "--campaign must match $name_re"
[[ "$target" =~ $name_re ]] || die "--target must match $name_re"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || [ "$commit" = unknown ] || die "--commit must be 40 hex digits or unknown"
case "$dirty" in true | false | unknown) ;; *) die "--dirty must be true, false or unknown" ;; esac
case "$counted" in true | false) ;; *) die "--counted must be true or false" ;; esac
[ -z "$runner" ] || [[ "$runner" =~ $name_re ]] || die "--runner must match $name_re"
[ -z "$preset" ] || [[ "$preset" =~ $name_re ]] || die "--preset must match $name_re"
[ -z "$uncounted" ] || [[ "$uncounted" =~ $reason_re ]] ||
  die "--uncounted: only letters, digits, spaces and ._:,()/- (at most 200)"

run_json=null
binary_sha=""
if [ -n "$run" ]; then
  [ -z "$seconds$workers$binary" ] && [ ${#ids[@]} -eq 0 ] ||
    die "--run takes the seconds, workers, binary and crashes from the summary; give none of them"
  [ -f "$run" ] || die "--run: no such file"
  run_json="$(jq -c . "$run" 2>/dev/null)" || die "--run: not JSON"
  # A summary is numbers, names and hashes. Any other string is refused, so no text of an input
  # can ride in it.
  jq -e 'type == "object" and ([.. | strings | select(test("^[A-Za-z0-9._-]*$") | not)] | length == 0)' \
    <<<"$run_json" >/dev/null || die "--run: the summary holds a value that is not a name, number or hash"
  seconds="$(jq -r '.seconds // empty' <<<"$run_json")"
  workers="$(jq -r '.workers // ((.fork // 0) | if . > 0 then . else 1 end)' <<<"$run_json")"
  binary_sha="$(jq -r '.binary_sha256 // empty' <<<"$run_json")"
  while IFS= read -r h; do
    [ -n "$h" ] && ids+=("$h")
  done < <(jq -r '.crash_sha256 // [] | .[]' <<<"$run_json")
  lf_exit="$(jq -r '.libfuzzer_exit // empty' <<<"$run_json")"
  lf_crashes="$(jq -r '.crashes // empty' <<<"$run_json")"
  if [ -z "$uncounted" ] && [ -n "$lf_exit" ] && [ "$lf_exit" != 0 ] && [ "${lf_crashes:-0}" = 0 ]; then
    uncounted="libFuzzer exit $lf_exit with no crash file"
  fi
elif [ "$binary_given" = yes ]; then
  if [ -f "$binary" ]; then
    binary_sha="$(sha256 "$binary")"
  fi
fi
[[ "$seconds" =~ ^[0-9]+$ ]] || die "seconds must be a whole number (--seconds or the summary)"
[[ "$workers" =~ ^[0-9]+$ ]] && [ "$workers" -ge 1 ] || die "workers must be a whole number >= 1"
if [ -n "$binary_sha" ]; then
  [[ "$binary_sha" =~ $hex64_re ]] || die "the binary hash is not 64 hex digits"
elif [ -z "$uncounted" ]; then
  uncounted="no binary hash"
fi
for h in ${ids[@]+"${ids[@]}"}; do
  [[ "$h" =~ $hex64_re ]] || die "a crash id is not 64 lowercase hex digits"
done

# measured seconds x workers / 3600, 4 decimals
cpu_hours="$(awk -v s="$seconds" -v w="$workers" 'BEGIN { printf "%.4f", s * w / 3600 }')"
ids_json="$(printf '%s\n' ${ids[@]+"${ids[@]}"} | jq -R . | jq -cs 'map(select(length > 0)) | unique')"

record="$(
  jq -cn \
    --arg tool "$tool" --arg campaign "$campaign" --arg target "$target" --arg commit "$commit" \
    --arg dirty "$dirty" --arg cpu "$cpu_hours" --argjson seconds "$seconds" \
    --argjson workers "$workers" --arg counted "$counted" --arg uncounted "$uncounted" \
    --argjson ids "$ids_json" --arg binary "$binary_sha" --arg runner "$runner" \
    --arg preset "$preset" --argjson run "$run_json" '
    {schema: "ep9-fuzz/v1", tool: $tool, campaign: $campaign, target: $target}
    + (if $preset == "" then {} else {preset: $preset} end)
    + {commit: $commit}
    + (if $dirty == "unknown" then {} else {dirty: ($dirty == "true")} end)
    + {cpu_hours: ($cpu | tonumber), seconds: $seconds, workers: $workers,
       counted: ($counted == "true" and $uncounted == "")}
    + (if $counted == "true" and $uncounted != "" then {uncounted_reason: $uncounted} else {} end)
    + {crashes: ($ids | map({id: ., state: "open"})),
       binary_sha256: (if $binary == "" then null else $binary end)}
    + (if $runner == "" then {} else {runner: $runner} end)
    + (if $run == null then {} else {run: $run} end)'
)"
# Last guard: every string of the record is a name, a hash or a fixed text.
jq -e '[.. | strings | select(test("^[A-Za-z0-9 ._:,()/-]*$") | not)] | length == 0' \
  <<<"$record" >/dev/null || die "the record holds a value outside the allowed characters"

if [ -n "$out" ]; then
  printf '%s\n' "$record" >"$out"
else
  printf '%s\n' "$record"
fi
