#!/usr/bin/env bash
# shellcheck disable=SC2154  # `root` is set by the caller
# Shared library for scripts/sync-<name>.sh. Source it; do not run it. Body as in docs/plan/08-repo.md section 5,
# plus pinned_tree (M5-P0): a stamp counts only when the checkout really is the pin plus the patches.
# usage: sync_dep <name> <git-url>; caller sets: set -euo pipefail; root=...
patch_hash() { { for p in "$1"/*.patch; do [ -e "$p" ] && cat "$p"; done; true; } | git hash-object --stdin; }
sync_dep() {
  local name=$1 url=$2 dir="$root/third_party/$1" pdir="$root/third_party/patches/$1" rev want
  rev="$(tr -d '[:space:]' < "$root/third_party/$name.commit")"
  want="$rev $(patch_hash "$pdir")"
  [ -d "$dir/.git" ] || { git init -q "$dir"; git -C "$dir" remote add origin "$url"; }
  git -C "$dir" config core.autocrlf false           # LF on every OS: .gitattributes stops at this work tree
  if [ "$(cat "$dir/.ep9-stamp" 2>/dev/null || true)" = "$want" ]; then
    pinned_tree "$dir" "$rev" "$pdir" && return 0
    echo "sync $name: the checkout is not $rev plus the patches (HEAD or a file changed); syncing again" >&2
  fi
  rm -f "$dir/.ep9-stamp"                            # no stamp while the checkout changes
  git -C "$dir" fetch -q --depth 1 origin "$rev"
  git -C "$dir" checkout -q -f --detach FETCH_HEAD   # drops patches from earlier runs
  git -C "$dir" clean -q -fd                         # keeps ignored target/
  for p in "$pdir"/*.patch; do [ -e "$p" ] || continue; git -C "$dir" apply --whitespace=nowarn "$p"; done
  echo "$want" > "$dir/.ep9-stamp"                   # re-runs when the pin OR the patch set changes
}
# pinned_tree DIR REV PDIR: HEAD is REV, and the work tree (tracked and untracked files, not the
# ignored ones such as target/, not the stamp) is REV's tree with the patches applied. It
# compares two tree ids made in scratch index files; the checkout's own index is not changed.
pinned_tree() {
  local dir=$1 rev=$2 pdir=$3 idx="$1/.git/ep9-sync-index" p want got
  [ "$(git -C "$dir" rev-parse -q --verify 'HEAD^{commit}' 2>/dev/null || true)" = "$rev" ] || return 1
  rm -f "$idx" "$idx.w"
  # want: REV's tree plus the patches (git apply --cached applies them in the scratch index)
  GIT_INDEX_FILE="$idx" git -C "$dir" read-tree "$rev" || return 1
  for p in "$pdir"/*.patch; do
    [ -e "$p" ] || continue
    GIT_INDEX_FILE="$idx" git -C "$dir" apply --cached --whitespace=nowarn "$p" 2>/dev/null || { rm -f "$idx"; return 1; }
  done
  want="$(GIT_INDEX_FILE="$idx" git -C "$dir" write-tree)" || return 1
  # got: the work tree, staged into a copy of the checkout's index (its stat data skips the
  # files that did not change since the last sync)
  cp "$dir/.git/index" "$idx.w" 2>/dev/null || true
  GIT_INDEX_FILE="$idx.w" git -C "$dir" add -A -- . ':(exclude).ep9-stamp' || { rm -f "$idx" "$idx.w"; return 1; }
  got="$(GIT_INDEX_FILE="$idx.w" git -C "$dir" write-tree)" || return 1
  rm -f "$idx" "$idx.w"
  [ "$want" = "$got" ]
}
