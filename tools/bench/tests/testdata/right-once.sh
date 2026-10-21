#!/bin/sh
# A fake engine for tests: its first run prints the right answer and every later run a wrong one
# ("12345"), like an engine whose bug shows only in the runs that are counted. The right answer is
# the `const want = ...` value of the script (its last argument), as in cross/testdata.
#   RIGHT_ONCE_STATE=FILE  counts the runs (`--version` is not a run)
if [ "$1" = "--version" ]; then
  echo "right-once 0.0"
  exit 0
fi
n=$(cat "$RIGHT_ONCE_STATE" 2>/dev/null || echo 0)
echo $((n + 1)) > "$RIGHT_ONCE_STATE"
if [ "$n" != 0 ]; then
  echo "12345"
  exit 0
fi
for a; do f=$a; done
sed -n 's/^const want = "\{0,1\}\([^";]*\)"\{0,1\};.*$/\1/p' "$f"
