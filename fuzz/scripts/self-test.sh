#!/usr/bin/env bash
# Self-test of the fuzz output rule (plan v9 V9-A17, plan v10 rule 14) and of the ep9-fuzz/v1
# records (plan v10 §2.7): a crash shows only the count and the SHA-256, and a record has every
# field and no input byte.
#
# First, with no cargo (jq only): record.sh's field rules and refusals, and the Fuzzilli path:
# infra/fuzz/triage.sh group ids and infra/fuzz/run.sh's record, with a stub FuzzilliCli that
# writes crash files holding a secret program text (no fuzzer runs). Then, with a stub `cargo`,
# a libFuzzer slow-unit file is not counted as a crash, and a run cut off before its summary
# still gives its crash ids (run.sh crashes). Then it builds frontend_compile with the
# `fixture-crash` feature (the target panics on an input that starts with "EP9-FIXTURE-CRASH"
# and prints the input), replays one fixture input with -runs=0 (no mutation), and checks that
#   - run.sh exits 1 and reports exactly 1 crash file with the fixture's SHA-256,
#   - the log does hold the input (so the fixture really leaked it there),
#   - run.sh's output holds no part of the input: not the text, not libFuzzer's hex bytes and
#     not its Base64 line, not cargo-fuzz's Debug bytes,
#   - the record that record.sh writes from that run (as fuzz.yml does) has every field, its
#     crash id is the fixture's SHA-256, and it holds no part of the input either.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
infra="$here/../../infra/fuzz"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/in"
secret="EP9-FIXTURE-CRASH-$RANDOM$RANDOM-secret"
printf '\001%s' "$secret" >"$work/in/fixture"
fail() {
  echo "fuzz self-test: FAILED: $1"
  exit 1
}
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}
command -v jq >/dev/null 2>&1 || fail "jq is required (record.sh writes the record with it)"
want="$(sha256 "$work/in/fixture")"
b64="$(base64 <"$work/in/fixture" | tr -d '\n')"
hex="0x45,0x50,0x39,0x2d,0x46,0x49,0x58"        # "EP9-FIX" as libFuzzer prints bytes
dbg="69, 80, 57, 45, 70, 73, 88"                # "EP9-FIX" as cargo-fuzz prints Debug bytes
# no_leak <what> <file>: the file holds no part of the secret input.
no_leak() {
  local leak
  for leak in "$secret" "${secret#EP9-FIXTURE-CRASH-}" "$b64" "$hex" "$dbg" "EP9-FIXTURE"; do
    if grep -qF -- "$leak" "$2"; then
      fail "$1 holds input bytes ($leak)"
    fi
  done
}
# has_fields <what> <record file>: one ep9-fuzz/v1 JSON line with every field of the m1-exit
# reader (tools/xtask/src/m1_exit.rs item 5) and binary_sha256.
has_fields() {
  [ "$(wc -l <"$2" | tr -d ' ')" = 1 ] || fail "$1: the record is not one line"
  jq -e '.schema == "ep9-fuzz/v1"
    and (.tool | IN("cargo-fuzz", "fuzzilli")) and (.campaign | type == "string")
    and (.target | type == "string") and (.commit | test("^[0-9a-f]{40}$"))
    and (.dirty | type == "boolean") and (.cpu_hours | type == "number")
    and (.counted | type == "boolean") and (.crashes | type == "array")
    and all(.crashes[]; (.id | test("^[0-9a-f]{64}$")) and .state == "open")
    and (.binary_sha256 | test("^[0-9a-f]{64}$"))' "$2" >/dev/null ||
    fail "$1: the record misses a field or has a wrong type: $(cat "$2")"
}

# 1. record.sh rules (explicit values).
commit="$(printf '%040d' 7 | tr 0 a)"
rec() {
  bash "$here/record.sh" --tool cargo-fuzz --campaign gh1-1-frontend_compile \
    --target frontend_compile --commit "$commit" "$@"
}
rec --dirty false --counted true --seconds 7200 --workers 4 --binary "$work/in/fixture" \
  --crash-file "$work/in/fixture" --crash-id "$want" --out "$work/r1.json" || fail "record.sh exit $?"
has_fields "record.sh" "$work/r1.json"
no_leak "record.sh's record" "$work/r1.json"
jq -e --arg want "$want" '.cpu_hours == 8 and .seconds == 7200 and .workers == 4
  and .counted == true and [.crashes[].id] == [$want]' "$work/r1.json" >/dev/null ||
  fail "record.sh: cpu_hours is not seconds x workers / 3600, or the crash ids are not one id"
rec --dirty unknown --counted true --seconds 60 --workers 1 --binary "$work/in/fixture" \
  --out "$work/r2.json" || fail "record.sh (dirty unknown) exit $?"
jq -e 'has("dirty") | not' "$work/r2.json" >/dev/null ||
  fail "record.sh: an unknown dirty state wrote a dirty field"
rec --dirty false --counted true --seconds 60 --workers 1 --binary "$work/in/fixture" \
  --uncounted "FuzzilliCli exit 1" --out "$work/r3.json" || fail "record.sh (uncounted) exit $?"
jq -e '.counted == false and .uncounted_reason == "FuzzilliCli exit 1"' "$work/r3.json" >/dev/null ||
  fail "record.sh: --uncounted did not write counted false"
rec --dirty false --counted true --seconds 60 --workers 1 --binary "$work/no-such-binary" \
  --out "$work/r4.json" || fail "record.sh (no binary) exit $?"
jq -e '.counted == false and .binary_sha256 == null' "$work/r4.json" >/dev/null ||
  fail "record.sh: a record with no binary hash is counted"
printf '{"tool":"cargo-fuzz","target":"t","mode":"60","seconds":60,"fork":4,"workers":4,"binary_sha256":"%s","libfuzzer_exit":1,"executed_units":9,"crashes":0,"crash_sha256":[],"slow_units":0,"slow_sha256":[]}\n' \
  "$want" >"$work/abnormal.json"
rec --dirty false --counted true --run "$work/abnormal.json" --out "$work/r5.json" ||
  fail "record.sh (abnormal run) exit $?"
jq -e '.counted == false and .cpu_hours == 0.0667 and .run.executed_units == 9' "$work/r5.json" >/dev/null ||
  fail "record.sh: a libFuzzer exit 1 with no crash file is counted, or the summary numbers are lost"
# Refusals (exit 2, nothing written): values that could carry input text.
refused() { # <what> <record.sh args>...
  local what="$1" rc
  shift
  rm -f "$work/bad.json"
  bash "$here/record.sh" --tool cargo-fuzz --target t --commit "$commit" --dirty false \
    --counted false "$@" --out "$work/bad.json" 2>/dev/null
  rc=$?
  [ "$rc" -eq 2 ] || fail "record.sh accepted $what (exit $rc)"
  [ ! -e "$work/bad.json" ] || fail "record.sh wrote a record for $what"
}
one=(--seconds 1 --workers 1)
refused "a quote in the campaign" --campaign 'a"b' "${one[@]}"
refused "a commit that is not 40 hex" --campaign c --commit HEAD "${one[@]}"
refused "a crash id of 65 hex" --campaign c --crash-id "${want}0" "${one[@]}"
refused "a runner with a slash" --campaign c --runner a/b "${one[@]}"
refused "input text in the reason" --campaign c --uncounted "$secret\"" "${one[@]}"
jq -c --arg s "$secret \"x" '.mode = $s' "$work/abnormal.json" >"$work/leaky.json"
refused "input text in the run summary" --campaign c --run "$work/leaky.json"
echo "fuzz self-test: record.sh ok (every field, measured CPU-h, no dirty field when unknown, refusals)"

# 2. The Fuzzilli path: triage ids and infra/fuzz/run.sh's record, with a stub FuzzilliCli.
#    Two crash files with different program text and the same bug (only numbers differ in the
#    panic text) are one group id, and so is a third whose frames moved to other lines (an
#    edit); a fourth with another signal is a second id.
crash_js() { # <file> <program text> <signal> <index> [<line of the first frame>]
  cat >"$1" <<JS
var s = "$2";
crash(s);
// CRASH INFO
// ==========
// TERMSIG: $3
// STDERR:
// ep9: internal error: index $4 out of range (this is an ep9 bug)
// stack backtrace:
//    0: std::panicking::begin_panic
//    1: ep9_vm::interp::run::h0123456789abcdef
//              at /src/crates/ep9_vm/src/interp/mod.rs:${5:-100}:5
//    2: ep9_vm::isolate::Isolate::eval::h0123456789abcdef
//              at /src/crates/ep9_vm/src/isolate.rs:50:9
//    3: ep9_cli::main::h0123456789abcdef
//              at /src/crates/ep9_cli/src/main.rs:10:1
// STDOUT:
// $2
// ARGS: ep9 --reprl
JS
}
mkdir -p "$work/crashes"
crash_js "$work/crashes/a.js" "$secret" 6 12
crash_js "$work/crashes/b.js" "$secret-two" 6 977
crash_js "$work/crashes/c.js" "$secret" 11 3
crash_js "$work/crashes/d.js" "$secret-three" 6 5 140
bash "$infra/triage.sh" "$work/crashes" >"$work/triage.txt" 2>&1 || fail "triage.sh exit $?"
no_leak "triage.sh's output" "$work/triage.txt"
bash "$infra/triage.sh" --ids "$work/crashes" >"$work/ids.txt" || fail "triage.sh --ids exit $?"
[ "$(wc -l <"$work/ids.txt" | tr -d ' ')" = 2 ] || fail "triage.sh --ids: not 2 group ids for 2 bugs"
grep -qvE '^[0-9a-f]{64}$' "$work/ids.txt" && fail "triage.sh --ids printed a line that is not an id"
printf '6\tindex N out of range\tep9_vm::interp::run@mod.rs < ep9_vm::isolate::Isolate::eval@isolate.rs < ep9_cli::main@main.rs' \
  >"$work/key.txt"
grep -qx "$(sha256 "$work/key.txt")" "$work/ids.txt" ||
  fail "triage.sh --ids: the SIGABRT group id is not the SHA-256 of its key line"
while IFS= read -r id; do
  grep -q "^  id:     $id$" "$work/triage.txt" || fail "triage.sh: group id $id is not printed in its block"
done <"$work/ids.txt"

mkdir -p "$work/fz"
cat >"$work/fz/FuzzilliCli" <<'STUB'
#!/usr/bin/env bash
# A stub: copies the crash files and prints two stats blocks; no fuzzer runs.
for a in "$@"; do case "$a" in --storagePath=*) st="${a#--storagePath=}" ;; esac; done
mkdir -p "$st/crashes"
cp "$STUB_CRASHES"/*.js "$st/crashes/"
for c in 1.00 2.50; do
  printf 'Uptime: 0d 0h 1m 0s\nCoverage: %s%%\nCrashes Found: 4\nTotal Execs: 100\n' "$c"
done
STUB
printf '#!/bin/sh\nexit 0\n' >"$work/fz/ep9"
chmod +x "$work/fz/FuzzilliCli" "$work/fz/ep9"
printf 'commit=%s\ndirty=false\n' "$commit" >"$work/fz/SOURCE"
STUB_CRASHES="$work/crashes" EP9="$work/fz/ep9" FUZZILLI="$work/fz/FuzzilliCli" JOBS=4 \
  FUZZ_BOX=box1 FUZZ_SESSION=s1 EP9_SOURCE="$work/fz/SOURCE" \
  bash "$infra/run.sh" 1 "$work/fz/out" >"$work/fz/run.txt" 2>&1
rc=$?
[ "$rc" -eq 0 ] || { cat "$work/fz/run.txt"; fail "infra/fuzz/run.sh (stub) exit $rc, expected 0"; }
no_leak "infra/fuzz/run.sh's output" "$work/fz/run.txt"
has_fields "infra/fuzz/run.sh" "$work/fz/out/record.json"
no_leak "infra/fuzz/run.sh's record" "$work/fz/out/record.json"
jq -e --slurpfile ids <(jq -R . "$work/ids.txt") '.tool == "fuzzilli" and .target == "reprl"
  and (.campaign | test("^fz-[0-9]{8}-box1-s1$")) and .dirty == false and .counted == false
  and .workers == 4 and ([.crashes[].id] == ($ids | sort))' "$work/fz/out/record.json" >/dev/null ||
  fail "infra/fuzz/run.sh: the record's campaign, crash ids or flags are wrong: $(cat "$work/fz/out/record.json")"
rm -f "$work/fz/SOURCE"
STUB_CRASHES="$work/crashes" EP9="$work/fz/ep9" FUZZILLI="$work/fz/FuzzilliCli" JOBS=4 \
  COUNTED=true EP9_SOURCE="$work/fz/SOURCE" bash "$infra/run.sh" 1 "$work/fz/out2" >/dev/null 2>&1 ||
  fail "infra/fuzz/run.sh (no SOURCE) exit $?"
jq -e '.commit == "unknown" and (has("dirty") | not)' "$work/fz/out2/record.json" >/dev/null ||
  fail "infra/fuzz/run.sh: an image with no SOURCE did not write commit unknown and no dirty field"
# The effective preset (Codex r1 reprl2): the record names the preset the REPRL child runs,
# EP9_REPRL_PRESET or else the build's own (line 2 of `ep9 --version`), apart from the target
# (the binary). A jit build run with EP9_REPRL_PRESET=jitless is preset jitless, target reprl.
# A setting the child refuses (jit on a jitless build, any other value) is exit 2 before
# Fuzzilli starts, and no record is written.
printf 'commit=%s\ndirty=false\n' "$commit" >"$work/fz/SOURCE"
printf '#!/bin/sh\necho "ep9 0.0.0"\necho "features jit,become; preset jit"\n' >"$work/fz/ep9-jit"
printf '#!/bin/sh\necho "ep9 0.0.0"\necho "features none; preset jitless"\n' >"$work/fz/ep9-jitless"
chmod +x "$work/fz/ep9-jit" "$work/fz/ep9-jitless"
preset_run() { # <ep9 stub> <out dir> [<EP9_REPRL_PRESET>]
  (
    if [ $# -ge 3 ]; then export EP9_REPRL_PRESET="$3"; else unset EP9_REPRL_PRESET; fi
    STUB_CRASHES="$work/crashes" EP9="$1" FUZZILLI="$work/fz/FuzzilliCli" JOBS=1 \
      EP9_SOURCE="$work/fz/SOURCE" bash "$infra/run.sh" 1 "$2" >"$2.txt" 2>&1
  )
}
for c in "jit reprl jit" "jit reprl jitless jitless" "jitless reprl-jitless jitless" \
  "jitless reprl-jitless jitless jitless"; do
  # shellcheck disable=SC2086 # split on purpose: the case words
  set -- $c
  dir="$work/fz/p-$1-${4:-unset}"
  if [ $# -ge 4 ]; then preset_run "$work/fz/ep9-$1" "$dir" "$4"; else preset_run "$work/fz/ep9-$1" "$dir"; fi
  rc=$?
  [ "$rc" -eq 0 ] || { cat "$dir.txt"; fail "infra/fuzz/run.sh ($1 build, setting ${4:-unset}) exit $rc"; }
  jq -e --arg t "$2" --arg p "$3" '.target == $t and .preset == $p' "$dir/record.json" >/dev/null ||
    fail "infra/fuzz/run.sh ($1 build, setting ${4:-unset}): want target $2, preset $3: $(cat "$dir/record.json")"
done
for c in "jitless jit" "jit hybrid" "jit JIT" "jit "; do
  # shellcheck disable=SC2086 # split on purpose: the case words
  set -- $c
  dir="$work/fz/bad-$1-${2:-empty}"
  preset_run "$work/fz/ep9-$1" "$dir" "${2:-}"
  rc=$?
  [ "$rc" -eq 2 ] || { cat "$dir.txt"; fail "infra/fuzz/run.sh ($1 build, setting '${2:-}') exit $rc, expected 2"; }
  [ ! -e "$dir/record.json" ] || fail "infra/fuzz/run.sh ($1 build, setting '${2:-}') wrote a record"
done
rec --dirty false --counted false --seconds 1 --workers 1 --preset 'jit less' >/dev/null 2>&1 &&
  fail "record.sh took a preset that is not a name"
echo "fuzz self-test: the record names the effective preset (EP9_REPRL_PRESET, else the build's own)"
echo "fuzz self-test: Fuzzilli record ok (triage group ids, campaign id, no program text)"

# 3. A slow unit is listed but is not a crash. A stub `cargo`, first on PATH, writes a
# `slow-unit-*` file where libFuzzer would (no fuzzer runs); run.sh must exit 0 with 0 crash
# files and 1 slow unit, also in the summary.
mkdir -p "$work/bin"
cat >"$work/bin/cargo" <<'STUB'
#!/usr/bin/env bash
for a in "$@"; do
  case "$a" in -artifact_prefix=*) printf 'slow' >"${a#-artifact_prefix=}slow-unit-0" ;; esac
done
echo "stat::number_of_executed_units: 1"
STUB
chmod +x "$work/bin/cargo"
out="$(PATH="$work/bin:$PATH" FUZZ_LOG_DIR="$work/slow-logs" FUZZ_SUMMARY="$work/slow.json" \
  bash "$here/run.sh" frontend_compile replay "$work/slow-corpus" 2>&1)"
rc=$?
echo "$out"
[ "$rc" -eq 0 ] || fail "a slow unit: run.sh exit $rc, expected 0"
echo "$out" | grep -q ': libFuzzer exit 0, 0 crash file(s), 1 slow unit(s)$' ||
  fail "a slow unit: no '0 crash file(s), 1 slow unit(s)' line"
grep -q '"crashes":0,' "$work/slow.json" && grep -q '"slow_units":1,' "$work/slow.json" ||
  fail "a slow unit: the summary does not say 0 crashes and 1 slow unit"

# 3b. A run cut off after a crash file and before its summary (fuzz.yml's Summary step then
# finds no summary). A stub `cargo` writes a crash file where libFuzzer would and then kills
# run.sh. `run.sh crashes <summary>` must still give that file's hash (from the run's start
# marker <summary>.started), and the record written as fuzz.yml's fallback writes it must hold
# it. A summary path that no run started with gives no hash, though the artifact dir holds a
# crash file: so an earlier run's crash (this self-test's fixture) never reaches a campaign
# record.
cat >"$work/bin/cargo" <<'STUB'
#!/usr/bin/env bash
for a in "$@"; do
  case "$a" in -artifact_prefix=*) printf 'cut' >"${a#-artifact_prefix=}crash-0" ;; esac
done
kill -TERM "$PPID"
STUB
printf 'cut' >"$work/cut-input"
cut_want="$(sha256 "$work/cut-input")"
(PATH="$work/bin:$PATH" FUZZ_LOG_DIR="$work/cut-logs" FUZZ_SUMMARY="$work/cut.json" FUZZ_FORK=4 \
  bash "$here/run.sh" frontend_compile 60 "$work/cut-corpus") >"$work/cut.txt" 2>&1
rc=$?
[ "$rc" -ne 0 ] && [ ! -e "$work/cut.json" ] || fail "a cut run: run.sh exit $rc, or it wrote a summary"
ids="$(bash "$here/run.sh" crashes "$work/cut.json")" || fail "a cut run: run.sh crashes exit $?"
[ "$ids" = "$cut_want" ] || fail "a cut run: run.sh crashes gave '$ids', not the crash file's hash"
cut_args=()
for h in $ids; do cut_args+=(--crash-id "$h"); done
bash "$here/record.sh" --tool cargo-fuzz --campaign gh0-1-frontend_compile --target frontend_compile \
  --commit "$commit" --dirty false --counted false --seconds 0 --workers 4 \
  --uncounted "no run summary: the campaign step did not end" ${cut_args[@]+"${cut_args[@]}"} \
  --out "$work/cut-record.json" || fail "a cut run: record.sh exit $?"
jq -e --arg want "$cut_want" '.schema == "ep9-fuzz/v1" and .dirty == false and .counted == false
  and .cpu_hours == 0 and [.crashes[].id] == [$want] and all(.crashes[]; .state == "open")' \
  "$work/cut-record.json" >/dev/null || fail "a cut run: the record lost the crash id: $(cat "$work/cut-record.json")"
ids="$(bash "$here/run.sh" crashes "$work/never-started.json")" || fail "run.sh crashes (no run) exit $?"
[ -z "$ids" ] || fail "run.sh crashes gave hashes for a summary path that no run started with"
echo "fuzz self-test: a cut run's crash reaches the record; no run, no crash ids"

# 4. The fixture crash, its output and its record.
FUZZ_FEATURES=fixture-crash bash "$here/run.sh" build frontend_compile >"$work/build.log" 2>&1 ||
  { cat "$work/build.log"; fail "build"; }
out="$(FUZZ_FEATURES=fixture-crash FUZZ_LOG_DIR="$work/logs" FUZZ_SUMMARY="$work/fixture.json" \
  bash "$here/run.sh" frontend_compile replay "$work/corpus" "$work/in" 2>&1)"
rc=$?
echo "$out"
[ "$rc" -eq 1 ] || fail "run.sh exit $rc, expected 1"
echo "$out" | grep -q ': libFuzzer exit [0-9]*, 1 crash file(s), 0 slow unit(s)$' || fail "no '1 crash file(s)' line"
echo "$out" | grep -q "crash sha256:$want$" || fail "the crash hash is not the fixture's"
grep -qF "$secret" "$work/logs/frontend_compile.log" || fail "the fixture did not print its input to the log"
printf '%s\n' "$out" >"$work/out.txt"
no_leak "run.sh output" "$work/out.txt"
# The record of that run, as fuzz.yml's Summary step writes it.
bash "$here/record.sh" --tool cargo-fuzz --campaign gh0-1-frontend_compile --target frontend_compile \
  --commit "$commit" --dirty false --counted false --runner self-test --run "$work/fixture.json" \
  --out "$work/fixture-record.json" || fail "record.sh --run exit $?"
has_fields "the fixture run" "$work/fixture-record.json"
no_leak "the fixture run's record" "$work/fixture-record.json"
jq -e --arg want "$want" '[.crashes[].id] == [$want] and .run.crashes == 1' \
  "$work/fixture-record.json" >/dev/null ||
  fail "the fixture run's record: the crash id is not the fixture's SHA-256"
cat "$work/fixture-record.json"
echo "fuzz self-test: ok (records; a slow unit is no crash; 1 crash file, hash only, no input bytes in the output or the record)"
