#!/usr/bin/env bash
# Self-test of run.sh's output rule (plan v9 V9-A17): a fixture crash shows only the count and
# the SHA-256. First, with a stub `cargo`, a libFuzzer slow-unit file is not counted as a
# crash. Then it builds frontend_compile with the `fixture-crash` feature (the target panics on
# an input that starts with "EP9-FIXTURE-CRASH" and prints the input), replays one fixture
# input with -runs=0 (no mutation), and checks that
#   - run.sh exits 1 and reports exactly 1 crash file with the fixture's SHA-256,
#   - the log does hold the input (so the fixture really leaked it there),
#   - run.sh's output holds no part of the input: not the text, not libFuzzer's hex bytes and
#     not its Base64 line, not cargo-fuzz's Debug bytes.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/in"
secret="EP9-FIXTURE-CRASH-$RANDOM$RANDOM-secret"
printf '\001%s' "$secret" >"$work/in/fixture"
fail() {
  echo "fuzz self-test: FAILED: $1"
  exit 1
}

# A slow unit is listed but is not a crash. A stub `cargo`, first on PATH, writes a
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

FUZZ_FEATURES=fixture-crash bash "$here/run.sh" build frontend_compile >"$work/build.log" 2>&1 ||
  { cat "$work/build.log"; fail "build"; }
out="$(FUZZ_FEATURES=fixture-crash FUZZ_LOG_DIR="$work/logs" \
  bash "$here/run.sh" frontend_compile replay "$work/corpus" "$work/in" 2>&1)"
rc=$?
echo "$out"
[ "$rc" -eq 1 ] || fail "run.sh exit $rc, expected 1"
echo "$out" | grep -q ': libFuzzer exit [0-9]*, 1 crash file(s), 0 slow unit(s)$' || fail "no '1 crash file(s)' line"
if command -v sha256sum >/dev/null 2>&1; then
  want="$(sha256sum "$work/in/fixture" | cut -d' ' -f1)"
else
  want="$(shasum -a 256 "$work/in/fixture" | cut -d' ' -f1)"
fi
echo "$out" | grep -q "crash sha256:$want$" || fail "the crash hash is not the fixture's"
grep -qF "$secret" "$work/logs/frontend_compile.log" || fail "the fixture did not print its input to the log"
b64="$(base64 <"$work/in/fixture" | tr -d '\n')"
hex="0x45,0x50,0x39,0x2d,0x46,0x49,0x58"        # "EP9-FIX" as libFuzzer prints bytes
dbg="69, 80, 57, 45, 70, 73, 88"                # "EP9-FIX" as cargo-fuzz prints Debug bytes
for leak in "$secret" "${secret#EP9-FIXTURE-CRASH-}" "$b64" "$hex" "$dbg" "EP9-FIXTURE"; do
  if echo "$out" | grep -qF -- "$leak"; then
    fail "run.sh output holds input bytes ($leak)"
  fi
done
echo "fuzz self-test: ok (a slow unit is no crash; 1 crash file, hash only, no input bytes in the output)"
