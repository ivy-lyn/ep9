#!/usr/bin/env bash
# Syncs the benchmark sources to their pins (docs/plan/08-repo.md section 1, the bench-*.commit rows).
# usage: bash scripts/sync-bench.sh [bench-octane] [bench-v8v7] [bench-jetstream]   (default: all)
# One pin file per repo, because sync_dep reads third_party/<name>.commit:
#   bench-octane     chromium/octane, Octane 2.0 (archived 2017; Octane-7 kernels, TS workload)
#   bench-v8v7       boa-dev/data branch "benchmarks": bench/bench-v8/combined.js (V8 suite v7,
#                    the copy that Boa's CI and the research runs use [R07])
#   bench-jetstream  WebKit/JetStream branch JetStream3.0 (JetStream 3.0 JS workloads)
# Patches: third_party/patches/<name>/*.patch. A second run with the same pins and patches does nothing.
# perf CI v0 needs bench-v8v7 only (richards and deltablue; tools/bench/README.md "perf CI v0").
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$root/scripts/sync-lib.sh"
want=("$@"); [ ${#want[@]} -gt 0 ] || want=(bench-octane bench-v8v7 bench-jetstream)
for n in "${want[@]}"; do
  case "$n" in
    bench-octane | bench-v8v7 | bench-jetstream) ;;
    *) echo "sync-bench: unknown repo $n (known: bench-octane bench-v8v7 bench-jetstream)" >&2; exit 2 ;;
  esac
done
has() { local n; for n in "${want[@]}"; do [ "$n" = "$1" ] && return 0; done; return 1; }

if has bench-octane; then sync_dep bench-octane https://github.com/chromium/octane.git; fi
if has bench-v8v7; then sync_dep bench-v8v7 https://github.com/boa-dev/data.git; fi
if has bench-jetstream; then
  js="$root/third_party/bench-jetstream"
  before="$(cat "$js/.ep9-stamp" 2> /dev/null || true)"
  sync_dep bench-jetstream https://github.com/WebKit/JetStream.git

  # JetStream keeps a few files as zlib .z files. cli.js inflates them with a Wasm zlib, so a shell
  # without WebAssembly (node --jitless, d8 --jitless, ep9 before Wasm) must run cli.js with
  # --no-prefetch, which reads inflated copies (what upstream's `npm run decompress` writes).
  # Upstream git-ignores the copies, so `git clean` keeps them: inflate again after each new sync.
  missing=0
  while IFS= read -r z; do [ -f "$js/${z%.z}" ] || missing=1; done < <(git -C "$js" ls-files '*.z')
  if [ "$before" != "$(cat "$js/.ep9-stamp")" ] || [ "$missing" = 1 ]; then
    command -v node > /dev/null || { echo "sync-bench: node is needed to inflate the JetStream .z files" >&2; exit 1; }
    (cd "$js" && git ls-files '*.z' | node -e '
      const fs = require("fs"), zlib = require("zlib");
      for (const f of fs.readFileSync(0, "utf8").split("\n").filter(Boolean))
        fs.writeFileSync(f.slice(0, -2), zlib.inflateSync(fs.readFileSync(f)));')
    echo "bench-jetstream: inflated $(git -C "$js" ls-files '*.z' | wc -l | tr -d ' ') .z files"
  fi
fi

for n in "${want[@]}"; do
  echo "$n: $(tr -d '[:space:]' < "$root/third_party/$n.commit") in $root/third_party/$n"
done
