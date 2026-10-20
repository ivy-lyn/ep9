#!/usr/bin/env bash
# CI scope: which jobs and targets one CI run uses (REMOTE-CI decision, docs/governance.md section 3).
# Run by the `scope` job of .github/workflows/ci.yml. Writes key=value lines to $GITHUB_OUTPUT
# (stdout when unset). Exit 0 always, unless a command fails.
#
# Env: EVENT (github.event_name), DRAFT (true/false), FULL (true when the PR has label ci:full or a
#      dispatch asks for full), BEFORE (push: github.event.before), SHA (github.sha), REPO, BRANCH,
#      COST_MODE (repo variable CI_COST_MODE: standard (default) or lean), GH_TOKEN (schedule only),
#      CHANGED_FILES_FILE (tests only: a file with one changed path per line).
#
#   event              targets (build-test, test262-parse)          spikes / sp-tsrs-win / perf
#   pull_request draft linux-x64                                     no / no / no
#   pull_request       linux-x64 linux-arm64 windows-x64 (lean:      on path match or ci:full /
#                      linux-x64); + macos-arm64 on path match.      on path match or ci:full / yes
#                      ci:full: all 4, spikes, tsrs, even for docs
#   push to main       linux-x64                                     no / no / yes
#   schedule           all 4 (skipped when main has not moved)       Mondays (UTC) / Mondays / yes
#   workflow_dispatch  all 4                                         yes / yes / yes
#   docs-only change   none (only this job runs)                     no / no / no
set -euo pipefail

mac='{"name":"macos-arm64","os":"macos-15"}'
lx='{"name":"linux-x64","os":"ubuntu-24.04"}'
la='{"name":"linux-arm64","os":"ubuntu-24.04-arm"}'
win='{"name":"windows-x64","os":"windows-2025"}'
all4="[$mac,$lx,$la,$win]"

out="${GITHUB_OUTPUT:-/dev/stdout}"
emit() { echo "$1=$2" >> "$out"; echo "scope: $1=$2" >&2; }

# Changed paths, one per line. "*" alone means "unknown: treat everything as changed".
changed_files() {
  if [ -n "${CHANGED_FILES_FILE:-}" ]; then cat "$CHANGED_FILES_FILE"; return; fi
  case "$EVENT" in
    pull_request)
      # actions/checkout gives the merge commit; parent 1 is the base branch tip.
      git diff --name-only HEAD^1 HEAD ;;
    push)
      if [ -z "${BEFORE:-}" ] || [ "$BEFORE" = 0000000000000000000000000000000000000000 ] ||
         ! git fetch -q --depth=1 origin "$BEFORE" 2> /dev/null; then
        echo "*"
      else
        git diff --name-only "$BEFORE" "$SHA"
      fi ;;
    *) echo "*" ;;
  esac
}

is_doc() {   # changes that cannot change a build, test or gate result
  case "$1" in
    docs/*|*.md|LICENSE-MIT|LICENSE-APACHE|THIRD-PARTY-LICENSE|.github/CODEOWNERS) return 0 ;;
  esac
  return 1
}
is_mac() {   # paths where macOS arm64 can differ from the other 3 targets
  case "$1" in
    crates/ep9_platform/*|spikes/*|rust-toolchain.toml|Cargo.lock|.cargo/*|scripts/*|\
    .gitattributes|.github/workflows/*|.github/actions/*|.github/scripts/*) return 0 ;;
  esac
  return 1
}
is_spike() { # inputs of spikes/*/gate.sh
  case "$1" in
    spikes/*|crates/ep9_platform/*|tools/xtask/*|rust-toolchain.toml|.github/workflows/ci.yml|\
    .github/actions/*|.github/scripts/ci-scope.sh) return 0 ;;
  esac
  return 1
}
is_tsrs() {  # inputs of the sp-tsrs-win job
  case "$1" in
    third_party/tsrs.commit|third_party/patches/tsrs/*|scripts/sync-tsrs.sh|scripts/sync-lib.sh|\
    .github/scripts/tsrs-smoke.sh|rust-toolchain.toml) return 0 ;;
  esac
  return 1
}

code=false mac_hit=false spike_hit=false tsrs_hit=false
while IFS= read -r f; do
  [ -n "$f" ] || continue
  if [ "$f" = "*" ]; then code=true mac_hit=true spike_hit=true tsrs_hit=true; continue; fi
  is_doc "$f" || code=true
  if is_mac "$f"; then mac_hit=true; fi
  if is_spike "$f"; then spike_hit=true; fi
  if is_tsrs "$f"; then tsrs_hit=true; fi
done < <(changed_files)

full="${FULL:-false}"
weekly=false
[ "$(date -u +%u)" = 1 ] && weekly=true

case "$EVENT" in
  pull_request)
    if [ "${DRAFT:-false}" = true ]; then
      targets="[$lx]" spikes=false tsrs=false perf=false
    else
      if [ "${COST_MODE:-standard}" = lean ]; then targets="[$lx]"; else targets="[$lx,$la,$win]"; fi
      if [ "$mac_hit" = true ]; then targets="[$mac,${targets#[}"; fi
      spikes="$spike_hit" tsrs="$tsrs_hit" perf=true
    fi
    if [ "$full" = true ]; then code=true targets="$all4" spikes=true tsrs=true perf=true; fi ;;
  push)
    # The PR run already covered the other targets, spikes and tsrs for this change. Main runs
    # Linux x64 for the per-commit perf baseline and the cache; the nightly run covers the rest.
    targets="[$lx]"; spikes=false; tsrs=false; perf=true ;;
  schedule)
    code=true targets="$all4" spikes="$weekly" tsrs="$weekly" perf=true
    # Skip the night when main has not moved since the last successful scheduled run.
    last="$(gh run list --repo "$REPO" --workflow ci.yml --event schedule --branch "$BRANCH" \
      --status success --limit 1 --json headSha --jq '.[0].headSha // ""' 2> /dev/null || true)"
    if [ -n "$last" ] && [ "$last" = "$SHA" ]; then
      echo "scope: main is still at $SHA, as in the last successful nightly; nothing to run" >&2
      code=false
    fi ;;
  *) # workflow_dispatch
    code=true targets="$all4" spikes=true tsrs=true perf=true ;;
esac

if [ "$code" = false ]; then targets="[]" spikes=false tsrs=false perf=false; fi
emit code "$code"
emit targets "$targets"
emit spikes "$spikes"
emit tsrs "$tsrs"
emit perf "$perf"
