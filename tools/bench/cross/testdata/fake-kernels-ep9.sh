#!/bin/sh
# Fake ep9 for the kernels.mjs tests: runs the file on FAKE_NODE (a real node), so it prints what
# node prints, except:
#   math-names.js            prints a Math without log and random (the probe of kernels.mjs)
#   FAKE_FAIL=<name>:<line>  for <name>.js: prints ep9's error form "<file>:<line>:5: Uncaught
#                            TypeError: undefined is not a function" on stderr and exits 1
#   FAKE_WRONG=<name>        for <name>.js: prints "Richards 2" and exits 0
if [ "$1" = "--version" ]; then
  echo "fake-ep9 0.0"
  exit 0
fi
f=$1
n=$(basename "$f" .js)
if [ "$n" = "math-names" ]; then
  echo "abs floor pow E PI"
  exit 0
fi
if [ "${FAKE_FAIL%%:*}" = "$n" ]; then
  echo "$f:${FAKE_FAIL#*:}:5: Uncaught TypeError: undefined is not a function" >&2
  echo "    at $f:${FAKE_FAIL#*:}:5" >&2
  exit 1
fi
if [ "$FAKE_WRONG" = "$n" ]; then
  echo "Richards 2"
  exit 0
fi
exec "$FAKE_NODE" "$f"
