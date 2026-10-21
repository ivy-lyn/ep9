#!/bin/sh
# A fake engine for tests: its first RIGHT_RUNS runs print the right answer and every later run a
# wrong one ("12345"), like an engine whose bug shows only in the runs that are counted. The right
# answer is the `const want = ...` value of the script (its last argument), as in cross/testdata.
#   RIGHT_RUNS=N       how many runs are right (default 1; 0: every run is wrong)
#   RIGHT_STATE=FILE   counts the runs (`--version` is not a run); remove it to start again
if [ "$1" = "--version" ]; then
  echo "right-then-wrong 0.0"
  exit 0
fi
n=$(cat "$RIGHT_STATE" 2>/dev/null || echo 0)
echo $((n + 1)) > "$RIGHT_STATE"
if [ "$n" -ge "${RIGHT_RUNS:-1}" ]; then
  echo "12345"
  exit 0
fi
for a; do f=$a; done
sed -n 's/^const want = "\{0,1\}\([^";]*\)"\{0,1\};.*$/\1/p' "$f"
