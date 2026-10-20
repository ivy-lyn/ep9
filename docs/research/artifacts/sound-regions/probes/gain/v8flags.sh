#!/bin/bash
# usage: v8flags.sh BENCHDIR REPS ; prints min peak round per (bench, flags)
B=$1; R=$2
for rep in $(seq 1 $R); do
 for b in nbody richards raytracer; do
  for f in "" "--no-track-field-types" "--no-turbo-loop-variable" "--no-turbo-load-elimination" "--no-turbo-inlining"; do
   out=$(node $f $B/$b-nofields.p.js 2>&1 | tr '\n' ' ')
   echo "$rep|$b|${f:-default}|$out"
  done
 done
done
