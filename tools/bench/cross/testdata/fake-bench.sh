#!/bin/sh
# Fake ep9-bench for run.test.mjs: runs the command (--warmup + --runs times, with FAKE_COUNTED=1
# set), checks each run's stdout against --expect-stdout like ep9-bench (exit 4 and no JSON on a
# mismatch), then writes an ep9-bench/v1 JSON with fixed counts to --out.
# The count is 1000 + the length of the script name, so rows differ per workload.
#   FAKE_BENCH_LOG=FILE   append "<engine name> seed=<EP9_HASH_SEED or unset> expect=<yes|no>"
out=
expect=
runs=1
warmup=0
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out=$2; shift 2 ;;
    --expect-stdout) expect=$2; shift 2 ;;
    --runs) runs=$2; shift 2 ;;
    --warmup) warmup=$2; shift 2 ;;
    --) shift; break ;;
    *) shift ;;
  esac
done
if [ -n "$FAKE_BENCH_LOG" ]; then
  echo "$(basename "$1") seed=${EP9_HASH_SEED-unset} expect=$([ -n "$expect" ] && echo yes || echo no)" >> "$FAKE_BENCH_LOG"
fi
i=0
while [ "$i" -lt $((warmup + runs)) ]; do
  got=$(FAKE_COUNTED=1 "$@") || { echo "fake-bench: the command failed" >&2; exit 1; }
  if [ -n "$expect" ] && [ "$got" != "$(cat "$expect")" ]; then
    echo "fake-bench: run $i printed \"$got\", want \"$(cat "$expect")\"" >&2
    exit 4
  fi
  i=$((i + 1))
done
for a; do f=$a; done
n=$((1000 + ${#f}))
cat > "$out" << EOF2
{ "schema": "ep9-bench/v1", "name": "x", "command": [], "counter": "time", "counter_status": "ok",
  "counter_note": null, "unit": "instructions", "target": "fake", "commit": null,
  "runs": [ { "instructions": $n, "wall_ns": 1000000, "max_rss_bytes": 4096 } ],
  "summary": { "instructions": { "min": $n, "median": $n, "max": $n }, "wall_ns": { "median": 1000000 } } }
EOF2
