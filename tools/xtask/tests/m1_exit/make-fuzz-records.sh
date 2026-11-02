#!/usr/bin/env bash
# Remakes the fuzz record fixtures of this directory with the producer, fuzz/scripts/record.sh
# (plan v10 section 3.7, F4): the m1-exit item 5 tests read records in the form the producer
# writes. Run from the repo root: bash tools/xtask/tests/m1_exit/make-fuzz-records.sh
# Every record names the placeholder commit `f4` x 20; the tests put the fake repo's HEAD there.
# No input byte, corpus or log goes in a record (plan rule 14): names, numbers and hashes only.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
rec=fuzz/scripts/record.sh
commit="$(printf 'f4%.0s' $(seq 20))"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
# A stand-in for the fuzzed binary: record.sh writes its SHA-256 only.
printf 'ep9 fixture binary\n' >"$tmp/ep9"
# Fuzzilli: 90000 s x 4 workers = 100 CPU-h, one campaign, clean and counted.
bash "$rec" --tool fuzzilli --campaign fz-fixture-1 --target ep9 --commit "$commit" \
  --dirty false --counted true --seconds 90000 --workers 4 --binary "$tmp/ep9" \
  --runner ep9-fuzz --out "$here/fuzz-fuzzilli.json"
# The same campaign id, made from a dirty tree: not counted (CX6-1).
bash "$rec" --tool fuzzilli --campaign fz-fixture-1 --target ep9 --commit "$commit" \
  --dirty true --counted true --seconds 90000 --workers 4 --binary "$tmp/ep9" \
  --runner ep9-fuzz --out "$here/fuzz-fuzzilli-dirty.json"
# cargo-fuzz in the fuzz.yml form (--run with a fuzz/scripts/run.sh summary): 21600 s x fork 4
# = 24 CPU-h.
printf '{"tool":"cargo-fuzz","target":"frontend_compile","mode":"fuzz","seconds":21600,"fork":4,"workers":4,"binary_sha256":"%s","libfuzzer_exit":0,"executed_units":123456,"crashes":0,"crash_sha256":[],"slow_units":0,"slow_sha256":[]}\n' \
  "$(printf 'ab%.0s' $(seq 32))" >"$tmp/summary.json"
bash "$rec" --tool cargo-fuzz --campaign gh1-1-frontend_compile --target frontend_compile \
  --commit "$commit" --dirty false --counted true --runner ubuntu-24.04 \
  --run "$tmp/summary.json" --out "$here/fuzz-cargo-fuzz.json"
