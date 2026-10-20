#!/bin/bash
# usage: run.sh <name> <tsconfig-or-dir> [oracle args...]
set -e
W="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${OUT:-$W/out}"; mkdir -p "$OUT"
name=$1; proj=$2; shift 2
/usr/bin/time -l $W/oracle/target/release/ep9_typecheck "$proj" $OUT/$name.jsonl "$@" 2> $OUT/$name.oracle.log
python3 -c "
import json,sys
for l in open('$OUT/$name.jsonl'):
    d=json.loads(l)
    if 'file' in d: print(d['file'])
" > $OUT/$name.files
$W/joiner/target/release/oxc_span_dump $OUT/$name.oxc.jsonl - < $OUT/$name.files 2> $OUT/$name.oxc.log
python3 $W/scripts/join.py $OUT/$name.jsonl $OUT/$name.oxc.jsonl $OUT/$name.report.json
grep -E 'maximum resident|real' $OUT/$name.oracle.log
cat $OUT/$name.oxc.log
