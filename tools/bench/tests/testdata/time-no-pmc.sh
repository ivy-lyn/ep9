#!/bin/sh
# A fake macOS `/usr/bin/time -l` for tests (EP9_BENCH_TIME). It prints what Apple's time.c prints
# on a host with no PMC access, for example a hosted macOS runner: time.c writes the
# "instructions retired" and "cycles elapsed" lines only for a count above 0, so they are absent.
#
# usage: time-no-pmc.sh -l [-o FILE] <command> [args...]
# The report goes to FILE, or to stderr. The exit status is the command's.
[ "$1" = "-l" ] || { echo "time-no-pmc.sh: needs -l" >&2; exit 2; }
shift
out=
if [ "$1" = "-o" ]; then
  out=$2
  shift 2
fi
"$@"
status=$?
report() {
  echo "        0.00 real         0.00 user         0.00 sys"
  for line in "1261568 maximum resident set size" "0 average shared memory size" \
    "0 average unshared data size" "0 average unshared stack size" "254 page reclaims" \
    "1 page faults" "0 swaps" "0 block input operations" "0 block output operations" \
    "0 messages sent" "0 messages received" "0 signals received" \
    "1 voluntary context switches" "10 involuntary context switches" \
    "1327344 peak memory footprint"; do
    printf '%20s  %s\n' "${line%% *}" "${line#* }"
  done
}
if [ -n "$out" ]; then report > "$out"; else report >&2; fi
exit "$status"
