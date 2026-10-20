#!/usr/bin/env bash
# Syncs third_party/test262 to the commit in third_party/test262.commit, plus
# third_party/patches/test262/*.patch. A second run with the same pin and patches does nothing.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$root/scripts/sync-lib.sh"
sync_dep test262 https://github.com/tc39/test262.git
echo "test262: $(tr -d '[:space:]' < "$root/third_party/test262.commit") in $root/third_party/test262"
