#!/usr/bin/env bash
# Syncs third_party/regress to the commit in third_party/regress.commit, plus
# third_party/patches/regress/*.patch (the RX0 fork, V9-D15; docs/plan/08-repo.md section 1).
# The root, spikes/spike-b and spikes/spike-c workspaces build regress from this checkout through
# their `[patch.crates-io]` path entry, so every cargo command there needs it first (a missing
# patch path fails the resolve step). CI runs it in .github/actions/setup-rust.
# A second run with the same pin and patches does nothing.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$root/scripts/sync-lib.sh"
sync_dep regress https://github.com/ridiculousfish/regress.git
echo "regress: $(tr -d '[:space:]' < "$root/third_party/regress.commit") in $root/third_party/regress"
