#!/usr/bin/env bash
# Syncs third_party/fuzzilli to the commit in third_party/fuzzilli.commit, plus
# third_party/patches/fuzzilli/*.patch (the ep9 profile, `--profile=ep9`), then builds Fuzzilli's
# FuzzilliCli and REPRLRun with SwiftPM in release mode. A second run skips the fetch; SwiftPM
# then finds the build up to date.
#
# Needs Swift >= 6.3 (Fuzzilli's Package.swift: swift-tools-version 6.3) on macOS 15+ or Linux
# (the infra/fuzz image). SwiftPM fetches swift-protobuf (exact), swift-collections and
# swift-algorithms (up to next minor) from GitHub; Fuzzilli has no Package.resolved.
#
# Build only: this script runs no fuzzer. On the Mac nothing else runs (plan v9 rule 23); the
# mutation run is infra/fuzz/run.sh on Cloudflare. REPRLRun (one script at a time, no mutation)
# checks an `ep9 --reprl` build by hand:
#   echo 'fuzzilli("FUZZILLI_PRINT", "hi")' | <bin>/REPRLRun target/release/ep9 --reprl
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$root/scripts/sync-lib.sh"
sync_dep fuzzilli https://github.com/googleprojectzero/fuzzilli.git
fz="$root/third_party/fuzzilli"
command -v swift >/dev/null || { echo "sync-fuzzilli: no swift on PATH (need Swift >= 6.3)" >&2; exit 1; }
(cd "$fz" && swift build -c release --product FuzzilliCli && swift build -c release --product REPRLRun)
bin="$(cd "$fz" && swift build -c release --show-bin-path)"
echo "FuzzilliCli: $bin/FuzzilliCli"
echo "REPRLRun:    $bin/REPRLRun"
