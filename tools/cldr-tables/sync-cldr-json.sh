#!/usr/bin/env bash
# Syncs tools/cldr-tables/cldr-json to the commit in tools/cldr-tables/cldr-json.commit (the
# commit of the cldr-json tag that names the CLDR release of the ICU4X compiled data; INTL-V2-D5).
# The checkout is git-ignored. It is sparse and partial: only the files that the generator reads
# (CLDR_PATHS) are fetched. A second run with the same pin does nothing.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dir="$here/cldr-json"
url=https://github.com/unicode-org/cldr-json.git
rev="$(tr -d '[:space:]' < "$here/cldr-json.commit")"
# The paths the generator reads (src/lib.rs, `read`), and the licence.
CLDR_PATHS=(
  /cldr-json/cldr-core/package.json
  /cldr-json/cldr-core/availableLocales.json
  /cldr-json/cldr-core/supplemental/numberingSystems.json
  /cldr-json/cldr-numbers-full/package.json
  '/cldr-json/cldr-numbers-full/main/*/numbers.json'
  /LICENSE
)
want="$rev $(printf '%s\n' "${CLDR_PATHS[@]}" | git hash-object --stdin)"
if [ -d "$dir/.git" ] && [ "$(cat "$dir/.git/ep9-stamp" 2>/dev/null || true)" = "$want" ] &&
  [ "$(git -C "$dir" rev-parse -q --verify 'HEAD^{commit}' 2>/dev/null || true)" = "$rev" ] &&
  [ -z "$(git -C "$dir" status --porcelain)" ]; then
  echo "cldr-json: $rev in $dir"
  exit 0
fi
[ -d "$dir/.git" ] || { git init -q "$dir"; git -C "$dir" remote add origin "$url"; }
git -C "$dir" config core.autocrlf false
rm -f "$dir/.git/ep9-stamp"
git -C "$dir" sparse-checkout set --no-cone "${CLDR_PATHS[@]}"
git -C "$dir" fetch -q --depth 1 --filter=blob:none origin "$rev"
git -C "$dir" checkout -q -f --detach FETCH_HEAD
git -C "$dir" clean -q -fdx
[ "$(git -C "$dir" rev-parse 'HEAD^{commit}')" = "$rev" ] || { echo "cldr-json: HEAD is not $rev" >&2; exit 1; }
echo "$want" > "$dir/.git/ep9-stamp"
echo "cldr-json: $rev in $dir"
