#!/usr/bin/env bash
# Syncs third_party/icu4x to the commit in third_party/icu4x.commit (the upstream commit of the
# crates.io icu_calendar 2.4.0 package), plus third_party/patches/icu4x/*.patch (the icu_calendar
# `until` month search; INTL W1 R2). The root and spikes/spike-c workspaces build icu_calendar
# from third_party/icu4x/components/calendar through their `[patch.crates-io]` path entry, so
# every cargo command there needs it first (a missing patch path fails the resolve step), as for
# scripts/sync-regress.sh. CI runs it in .github/actions/setup-rust.
# A second run with the same pin and patches does nothing.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$root/scripts/sync-lib.sh"
sync_dep icu4x https://github.com/unicode-org/icu4x.git
echo "icu4x: $(tr -d '[:space:]' < "$root/third_party/icu4x.commit") in $root/third_party/icu4x"
