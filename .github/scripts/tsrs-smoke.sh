#!/usr/bin/env bash
# SP-tsrs-win smoke tests (docs/plan/08-repo.md section 5 item 7): the built `tsrs` prints its
# version, accepts a correct file and reports TS2322 for a wrong one.
# usage: bash .github/scripts/tsrs-smoke.sh <path to tsrs or tsrs.exe>
set -euo pipefail
tsrs="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"
printf 'const x: number = 1;\nexport {};\n' > ok.ts
printf 'const x: number = "s";\nexport {};\n' > bad.ts
fail=0

v="$("$tsrs" --version)"
if [[ "$v" == Version* ]]; then echo "ok:   --version: $v"; else echo "FAIL: --version printed: $v"; fail=1; fi

if "$tsrs" --noEmit ok.ts; then echo "ok:   ok.ts has no errors"; else echo "FAIL: ok.ts gave exit $?"; fail=1; fi

set +e
out="$("$tsrs" --noEmit bad.ts 2>&1)"; code=$?
set -e
if [ "$code" -ne 0 ] && grep -q "error TS2322" <<< "$out"; then
  echo "ok:   bad.ts gave exit $code and TS2322"
else
  echo "FAIL: bad.ts gave exit $code: $out"; fail=1
fi
exit "$fail"
