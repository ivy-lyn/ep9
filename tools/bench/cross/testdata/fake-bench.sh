#!/bin/sh
# Fake ep9-bench for run.test.mjs: writes an ep9-bench/v1 JSON with fixed counts to --out.
# The count is 1000 + the length of the script name, so rows differ per workload.
out=
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out=$2; shift 2 ;;
    --) shift; break ;;
    *) shift ;;
  esac
done
for a; do f=$a; done
n=$((1000 + ${#f}))
cat > "$out" << EOF
{ "schema": "ep9-bench/v1", "name": "x", "command": [], "counter": "time", "counter_status": "ok",
  "counter_note": null, "unit": "instructions", "target": "fake", "commit": null,
  "runs": [ { "instructions": $n, "wall_ns": 1000000, "max_rss_bytes": 4096 } ],
  "summary": { "instructions": { "min": $n, "median": $n, "max": $n }, "wall_ns": { "median": 1000000 } } }
EOF
