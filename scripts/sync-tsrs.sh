#!/usr/bin/env bash
# Syncs third_party/tsrs to the commit in third_party/tsrs.commit, plus third_party/patches/tsrs/*.patch.
# Then it builds, outside the root workspace (docs/plan/08-repo.md section 5):
#   1) tsrs_cli in tsrs's own workspace, with tsrs's lock and toolchain (binary `tsrs`, for `ep9 check`)
#   2) tools/ep9-typecheck, only when that directory exists (stage A1h)
# A second run skips the fetch; cargo then finds the builds up to date.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$root/scripts/sync-lib.sh"
sync_dep tsrs https://github.com/oxc-project/tsrs.git
tsrs="$root/third_party/tsrs"; tc="$root/tools/ep9-typecheck"
chan="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$tsrs/rust-toolchain.toml")"   # 1.99.1 today
[ -n "$chan" ] || { echo "sync-tsrs: no channel in $tsrs/rust-toolchain.toml" >&2; exit 1; }
# "+$chan" wins over RUSTUP_TOOLCHAIN and the root rust-toolchain.toml. Each build gets its own
# target dir, so an exported CARGO_TARGET_DIR does not move the outputs.
# 1) `ep9 check`: tsrs_cli in tsrs's own workspace (its lock, profile, toolchain; no PGO/BOLT)
(cd "$tsrs" && CARGO_TARGET_DIR="$tsrs/target" cargo "+$chan" build --release --locked -p tsrs_cli)
echo "tsrs:          $tsrs/target/release/tsrs"
# 2) AOT type oracle: own empty [workspace]; start from tsrs's exact lock, add only ep9 entries
if [ -d "$tc" ]; then
  cp "$tsrs/Cargo.lock" "$tc/Cargo.lock"
  (cd "$tc" && export CARGO_TARGET_DIR="$tc/target" &&
    cargo "+$chan" update --workspace && cargo "+$chan" build --release --locked)
  echo "ep9-typecheck: $tc/target/release/ep9-typecheck"
else
  echo "ep9-typecheck: skipped ($tc does not exist before stage A1h)"
fi
