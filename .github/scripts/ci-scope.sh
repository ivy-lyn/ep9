#!/usr/bin/env bash
# The CI scope of a commit range (docs/plan/12-governance.md section 3; plan v9 V9-A18).
#
#   bash .github/scripts/ci-scope.sh <base> <head>
#   bash .github/scripts/ci-scope.sh --self-test
#
# Prints one line on stdout: docs_only=true or docs_only=false. The reasons go to stderr.
#
# It is an allow-list and it fails closed. docs_only=true only when every path that changed
# between merge-base(<base>, <head>) and <head> was added or modified (git status A or M), and
# each one is under docs/ or is a *.md file in the repo root, and none of them is:
#   - docs/adr/README.md     (m0-exit reads it from the live tree, also in its tests:
#                             tools/xtask/src/m0_exit.rs adr_index and its tests)
#   - under docs/measurements/ (m0-exit reads that dir from the live tree, also in its tests)
# Any other path (a *.md under crates/, tools/, spikes/, expectations/ or infra/ too: for example
# crates/ep9_builtins/TEMPLATES.md is a doctest source), a delete, a rename (with --no-renames git
# shows it as a delete plus an add), a type change, an empty range (an empty commit is a CI
# re-run, plan v9 rule 27), a base or head that git cannot find, no merge base, or any git error
# gives docs_only=false. A code reader of another docs/ path must be added to EXCLUDED below.
#
# Exit 0 with the line above; exit 2 on a usage error (and no line). --self-test builds a
# throwaway repo in a temp dir, runs this script on its fixture ranges and exits 0 only if every
# fixture prints its expected line.
set -u

# Paths that look like docs but are inputs of code or tests. An entry ending in / is a prefix.
EXCLUDED=(docs/adr/README.md docs/measurements/)

usage() {
  echo "usage: bash .github/scripts/ci-scope.sh <base> <head> | --self-test" >&2
  exit 2
}

# Fail closed: say why on stderr, print docs_only=false, exit 0.
closed() {
  echo "ci-scope: $1 -> docs_only=false" >&2
  echo "docs_only=false"
  exit 0
}

# 0 when path $1 may change in a docs-only range.
docs_path() {
  local p="$1" e
  for e in "${EXCLUDED[@]}"; do
    case "$e" in
      */) case "$p" in "$e"*) return 1 ;; esac ;;
      *) [ "$p" = "$e" ] && return 1 ;;
    esac
  done
  case "$p" in
    docs/?*) return 0 ;;
    */*) return 1 ;;
    ?*.md) return 0 ;;
  esac
  return 1
}

scope() {
  local base="$1" head="$2" b h mb status path n=0
  [ -n "$base" ] && [ -n "$head" ] || closed "empty base or head"
  case "$base$head" in -* | *[[:space:]]*) closed "base or head looks like an option or has a space" ;; esac
  case "$head" in -*) closed "head looks like an option" ;; esac
  b="$(git rev-parse --verify --quiet "$base^{commit}")" || closed "base $base is not a commit here"
  h="$(git rev-parse --verify --quiet "$head^{commit}")" || closed "head $head is not a commit here"
  mb="$(git merge-base "$b" "$h")" || closed "no merge base of $b and $h"
  # -z: one NUL after the status and one after the path, so no path is quoted or split.
  # A temp file keeps git's exit status (a pipe or a process substitution would lose it).
  list="$(mktemp)" || closed "mktemp failed"
  trap 'rm -f "$list"' EXIT
  git -c core.quotePath=false diff --no-renames --no-ext-diff --name-status -z "$mb" "$h" -- > "$list" \
    || closed "git diff $mb $h failed"
  while IFS= read -r -d '' status && IFS= read -r -d '' path; do
    n=$((n + 1))
    case "$status" in
      A | M) ;;
      *) closed "status $status of $path (only A and M can be docs-only)" ;;
    esac
    docs_path "$path" || closed "$path is not a docs-only path"
  done < "$list"
  [ "$n" -gt 0 ] || closed "no changed path in $mb..$h"
  echo "ci-scope: $n changed paths in $mb..$h, all docs-only -> docs_only=true" >&2
  echo "docs_only=true"
}

self_test() {
  local self tmp fail=0 c0
  self="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
  tmp="$(mktemp -d)" || { echo "self-test: mktemp failed" >&2; return 1; }
  # A clean git: no user or system config (no signing, hooks or templates), fixed identity.
  export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
  export GIT_AUTHOR_NAME=ci-scope GIT_AUTHOR_EMAIL=ci-scope GIT_COMMITTER_NAME=ci-scope GIT_COMMITTER_EMAIL=ci-scope
  (
    set -e
    cd "$tmp"
    git init -q -b main .
    mkdir -p docs/adr docs/measurements docs/plan crates/ep9_builtins/src tools/bench .github/workflows
    echo a > README.md
    echo a > docs/plan/12-governance.md
    echo a > docs/adr/README.md
    echo a > docs/adr/H16.md
    echo a > docs/measurements/m.md
    echo a > crates/ep9_builtins/TEMPLATES.md
    echo a > crates/ep9_builtins/src/lib.rs
    echo a > tools/bench/README.md
    echo a > .github/workflows/ci.yml
    git add -A && git commit -q -m base
  ) > /dev/null || { echo "self-test: fixture repo setup failed" >&2; rm -rf "$tmp"; return 1; }
  c0="$(git -C "$tmp" rev-parse HEAD)"

  # fixture <name> <expected true|false> <shell commands run in the repo, then committed>
  fixture() {
    local name="$1" want="$2" cmds="$3" head got
    git -C "$tmp" checkout -q --detach "$c0"
    (cd "$tmp" && eval "$cmds" && git add -A && git commit -q --allow-empty -m "$name") > /dev/null 2>&1 \
      || { echo "FAIL $name: fixture commit failed" >&2; fail=1; return; }
    head="$(git -C "$tmp" rev-parse HEAD)"
    got="$(cd "$tmp" && bash "$self" "$c0" "$head" 2> /dev/null)"
    if [ "$got" = "docs_only=$want" ]; then
      echo "PASS $name: $got"
    else
      echo "FAIL $name: got '$got', want docs_only=$want"
      fail=1
    fi
  }

  # The plan v9 section 2.7 acceptance: 3 ranges, then 5 that must be false.
  fixture "docs only" true 'echo b >> docs/plan/12-governance.md; echo b >> docs/adr/H16.md; echo b >> README.md; echo n > docs/new.md'
  fixture "code" false 'echo b >> crates/ep9_builtins/src/lib.rs'
  fixture "mixed" false 'echo b >> docs/plan/12-governance.md; echo b >> .github/workflows/ci.yml'
  fixture "crates/ep9_builtins/TEMPLATES.md alone" false 'echo b >> crates/ep9_builtins/TEMPLATES.md'
  fixture "docs/adr/README.md alone" false 'echo b >> docs/adr/README.md'
  fixture "a file under docs/measurements/" false 'echo n > docs/measurements/new.md'
  fixture "tools/bench/README.md alone" false 'echo b >> tools/bench/README.md'
  fixture "docs delete + code rename" false 'git rm -q docs/adr/H16.md; git mv crates/ep9_builtins/src/lib.rs crates/ep9_builtins/src/lib2.rs'
  # More fail-closed cases.
  fixture "docs rename alone" false 'git mv docs/adr/H16.md docs/adr/H16-new.md'
  fixture "docs delete alone" false 'git rm -q docs/plan/12-governance.md'
  fixture "empty commit (CI re-run)" false ':'
  fixture "root non-md file" false 'echo n > LICENSE'
  fixture "docs non-md file" true 'echo n > docs/plan/table.csv'
  fixture "mode change of a docs file" true 'chmod +x docs/adr/H16.md'
  fixture "path with spaces and a newline" true 'printf x > "docs/a b
c.md"'

  # Ranges that git cannot resolve.
  local got
  got="$(cd "$tmp" && bash "$self" 0000000000000000000000000000000000000000 "$c0" 2> /dev/null)"
  if [ "$got" = docs_only=false ]; then echo "PASS unknown base: $got"; else echo "FAIL unknown base: got '$got'"; fail=1; fi
  got="$(cd "$tmp" && bash "$self" "" "$c0" 2> /dev/null)"
  if [ "$got" = docs_only=false ]; then echo "PASS empty base: $got"; else echo "FAIL empty base: got '$got'"; fail=1; fi
  got="$(cd "$tmp" && bash "$self" --output=x "$c0" 2> /dev/null)"
  if [ "$got" = docs_only=false ] && [ ! -e "$tmp/x" ]; then echo "PASS option as base: $got"; else echo "FAIL option as base: got '$got'"; fail=1; fi
  git -C "$tmp" checkout -q --orphan other > /dev/null 2>&1
  (cd "$tmp" && git rm -rq --cached . && echo z > docs/z.md && git add docs/z.md && git commit -q -m other) > /dev/null 2>&1
  got="$(cd "$tmp" && bash "$self" "$c0" other 2> /dev/null)"
  if [ "$got" = docs_only=false ]; then echo "PASS no merge base: $got"; else echo "FAIL no merge base: got '$got'"; fail=1; fi

  rm -rf "$tmp"
  if [ "$fail" -eq 0 ]; then echo "ci-scope self-test: all fixtures pass"; else echo "ci-scope self-test: FAIL"; fi
  return "$fail"
}

case "$#:${1-}" in
  1:--self-test) self_test; exit $? ;;
  2:*) scope "$1" "$2" ;;
  *) usage ;;
esac
