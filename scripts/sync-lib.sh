#!/usr/bin/env bash
# shellcheck disable=SC2154  # `root` is set by the caller
# Shared library for scripts/sync-<name>.sh. Source it; do not run it. Body as in docs/plan/08-repo.md section 5,
# plus pinned_tree (M5-P0): a stamp counts only when the checkout really is the pin plus the patches.
# usage: sync_dep <name> <git-url>; caller sets: set -euo pipefail; root=...
#        fork_attest <name>...; caller sets: root=... (FORK-CHECKOUT-ATTEST; below)
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
  # The scratch index names hold the shell's pid: the runner and ep9 build scripts can run
  # fork_attest at the same time on one checkout (cargo runs build scripts in parallel).
  local dir=$1 rev=$2 pdir=$3 idx="$1/.git/ep9-sync-index.$$" p want got
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
# fork_attest NAME...: prints one line "NAME=STATE ..." (one pair per NAME, in order) for the
# checkout third_party/NAME of the repo at $root (FORK-CHECKOUT-ATTEST). STATE is
#   pinned  its .ep9-stamp is the pin plus the patch set and pinned_tree holds: the checkout is
#           what sync_dep makes, so a second sync would keep it;
#   stale   any other checkout (another HEAD, an old patch set, an edited or extra file, no
#           stamp): cargo would build code that the pin and the patches do not name;
#   absent  no checkout.
# Status 0 when every NAME is pinned, else 1. It changes nothing in the checkout. The one check
# of "the checkout that cargo builds is the pin plus the patches": the runner and ep9 build
# scripts (tools/test262/runner_build.rs) and kernels.mjs call it.
#   sh -c '. "$1/scripts/sync-lib.sh"; root=$1; fork_attest regress icu4x' sh <root>
# POSIX sh (dash runs it): the build scripts run it with sh.
fork_attest() {
  local name dir pdir rev state out='' rc=0
  for name in "$@"; do
    dir="$root/third_party/$name" pdir="$root/third_party/patches/$name" state=absent
    if [ -d "$dir/.git" ]; then
      state=stale
      rev="$(tr -d '[:space:]' < "$root/third_party/$name.commit" 2>/dev/null || true)"
      if [ -n "$rev" ] && [ "$(cat "$dir/.ep9-stamp" 2>/dev/null || true)" = "$rev $(patch_hash "$pdir")" ] &&
        pinned_tree "$dir" "$rev" "$pdir"; then
        state=pinned
      fi
    fi
    [ "$state" = pinned ] || rc=1
    out="$out${out:+ }$name=$state"
  done
  printf '%s\n' "$out"
  return "$rc"
}
