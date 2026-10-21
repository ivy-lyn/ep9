#!/bin/sh
# Fake JS engine for run.test.mjs. It prints what a correct engine prints for a cross workload:
# the `const want = ...` value of a T0 workload file, and nothing for the empty script.
# The engine's name is the name it is called by: the test makes one symlink per engine.
#   FAKE_LOG=FILE      append "<name> <script name>" for each call
#   FAKE_WRONG=n:w     engine n prints a wrong result for workload w
#   FAKE_FAIL=n:w      engine n exits 3 on workload w
#   FAKE_WRONG_COUNTED=n:w   engine n prints a wrong result for workload w, but only in a counted
#                      run (fake-bench.sh sets FAKE_COUNTED=1): right in the output check
name=$(basename "$0")
if [ "$1" = "--version" ]; then
  echo "fake $name 0.0"
  exit 0
fi
for a; do f=$a; done
w=$(basename "$f" .js)
if [ -n "$FAKE_LOG" ]; then echo "$name $w" >> "$FAKE_LOG"; fi
if [ "$FAKE_FAIL" = "$name:$w" ]; then
  echo "fake failure" >&2
  exit 3
fi
if [ "$FAKE_WRONG" = "$name:$w" ] || { [ -n "$FAKE_COUNTED" ] && [ "$FAKE_WRONG_COUNTED" = "$name:$w" ]; }; then
  echo "12345"
  exit 0
fi
want=$(sed -n 's/^const want = "\{0,1\}\([^";]*\)"\{0,1\};.*$/\1/p' "$f")
if [ -n "$want" ]; then echo "$want"; fi
exit 0
