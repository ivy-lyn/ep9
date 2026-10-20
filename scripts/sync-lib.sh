#!/usr/bin/env bash
# shellcheck disable=SC2154  # `root` is set by the caller
# Shared library for scripts/sync-<name>.sh. Source it; do not run it. Body as in docs/plan/08-repo.md section 5.
# usage: sync_dep <name> <git-url>; caller sets: set -euo pipefail; root=...
patch_hash() { { for p in "$1"/*.patch; do [ -e "$p" ] && cat "$p"; done; true; } | git hash-object --stdin; }
sync_dep() {
  local name=$1 url=$2 dir="$root/third_party/$1" pdir="$root/third_party/patches/$1" rev want
  rev="$(tr -d '[:space:]' < "$root/third_party/$name.commit")"
  want="$rev $(patch_hash "$pdir")"
  [ -d "$dir/.git" ] || { git init -q "$dir"; git -C "$dir" remote add origin "$url"; }
  [ "$(cat "$dir/.ep9-stamp" 2>/dev/null || true)" = "$want" ] && return 0
  git -C "$dir" fetch -q --depth 1 origin "$rev"
  git -C "$dir" checkout -q -f --detach FETCH_HEAD   # drops patches from earlier runs
  git -C "$dir" clean -q -fd                         # keeps ignored target/
  for p in "$pdir"/*.patch; do [ -e "$p" ] || continue; git -C "$dir" apply --whitespace=nowarn "$p"; done
  echo "$want" > "$dir/.ep9-stamp"                   # re-runs when the pin OR the patch set changes
}
