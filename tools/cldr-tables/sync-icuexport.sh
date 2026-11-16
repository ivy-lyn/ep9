#!/usr/bin/env bash
# Fetches the ICU export data release asset that tools/cldr-tables/icuexport.pin names into
# tools/cldr-tables/icuexport/ (git-ignored) and checks its sha256 (INTL-V2-D17 Form A; the input
# of `cldr-tables collator-search`). A second run with a file of the right hash does nothing.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dir="$here/icuexport"
read -r tag file sha < <(grep -v '^#' "$here/icuexport.pin" | grep -v '^[[:space:]]*$' | head -n 1)
[ -n "${sha:-}" ] || { echo "icuexport: no pin line in $here/icuexport.pin" >&2; exit 1; }
sum() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1 || sha256sum "$1" | cut -d' ' -f1; }
if [ -f "$dir/$file" ] && [ "$(sum "$dir/$file")" = "$sha" ]; then
  echo "icuexport: $file ($tag) in $dir"
  exit 0
fi
mkdir -p "$dir"
curl -sSLf --retry 2 -o "$dir/$file.part" "https://github.com/unicode-org/icu/releases/download/$tag/$file"
got="$(sum "$dir/$file.part")"
[ "$got" = "$sha" ] || { rm -f "$dir/$file.part"; echo "icuexport: $file has sha256 $got, the pin says $sha" >&2; exit 1; }
mv -f "$dir/$file.part" "$dir/$file"
echo "icuexport: $file ($tag) in $dir"
