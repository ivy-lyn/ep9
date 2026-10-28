#!/usr/bin/env bash
# Groups Fuzzilli crash files by cause (plan v9 §5.5): the panic text (numbers made "N") plus
# the top 3 ep9 frames of the backtrace (the ep9 profile sets RUST_BACKTRACE=1), or the signal
# alone when there is no panic text (a wild store, a trap). SIGKILL gets the tag "killed,
# probably out of memory": Fuzzilli counts its own timeout kill as a timeout, not a crash.
#
#   infra/fuzz/triage.sh [<crash dir or .js file>...]   default /work/fuzz-out/storage/crashes
#
# Reads the "CRASH INFO" footer that Fuzzilli writes at the end of each crash .js file
# (TERMSIG, STDERR). Prints one block per group, the largest first: the file count, the signal,
# the key and the file names. It never prints program text. Exit 0; 2 when no file is found.
set -euo pipefail

[ "$#" -gt 0 ] || set -- /work/fuzz-out/storage/crashes
files=()
for a in "$@"; do
  if [ -d "$a" ]; then
    while IFS= read -r f; do files+=("$f"); done < <(find "$a" -maxdepth 1 -name '*.js' | sort)
  elif [ -f "$a" ]; then
    files+=("$a")
  fi
done
[ "${#files[@]}" -gt 0 ] || { echo "triage.sh: no crash .js file in: $*" >&2; exit 2; }

# key <file>: "<signal>\t<panic text>\t<frame < frame < frame>". A frame is "<symbol>@<file
# name>:<line>" from the std backtrace text ("  12: <symbol>", then "      at <path>:<l>:<c>").
# Frames in the Rust library (a /rustc/ path) and the REPRL panic hook and fault injection
# (reprl.rs) are skipped.
key() {
  awk '
    # numbers(s): each number that does not end a name (EP9, u32) becomes "N".
    function numbers(s,    out, pre) {
      out = ""
      while (match(s, /0x[0-9a-fA-F]+|[0-9]+/)) {
        pre = substr(s, 1, RSTART - 1)
        out = out pre (pre ~ /[A-Za-z_]$/ ? substr(s, RSTART, RLENGTH) : "N")
        s = substr(s, RSTART + RLENGTH)
      }
      return out s
    }
    function take(sym, at,    f) {
      if (sym == "" || n >= 3) return
      if (at ~ /\/rustc\// || at ~ /\/ep9_cli\/src\/reprl\.rs:/) return
      if (at == "" && sym ~ /^(<?(std|core|alloc)::|__rust|_start|__libc)/) return
      f = at; sub(/^.*\//, "", f); sub(/:[0-9]+$/, "", f)
      fr = fr (n ? " < " : "") sym (f == "" ? "" : "@" f)
      n++
    }
    { sub(/^\/\/ ?/, "") }
    /^CRASH INFO$/ { info = 1 }
    !info { next }
    /^TERMSIG: / { sig = $2 }
    /^STDERR:$/ { err = 1; next }
    /^STDOUT:$/ { take(sym, ""); sym = ""; err = 0 }
    !err { next }
    /^ep9: internal error: / && msg == "" {
      msg = $0
      sub(/^ep9: internal error: /, "", msg)
      sub(/ \(this is an ep9 bug\)$/, "", msg)
      msg = numbers(msg)
    }
    /^ *[0-9]+: / {
      take(sym, "")
      sym = $0; sub(/^ *[0-9]+: /, "", sym); sub(/::h[0-9a-f]+$/, "", sym)
      next
    }
    /^ +at / && sym != "" {
      at = $0; sub(/^ +at /, "", at)
      take(sym, at); sym = ""
    }
    END { take(sym, ""); printf "%s\t%s\t%s\n", sig, msg, fr }
  ' "$1"
}

for f in "${files[@]}"; do
  printf '%s\t%s\n' "$(key "$f")" "$(basename "$f")"
done | sort | awk -F '\t' '
  function signame(s) {
    if (s == 4) return "SIGILL"; if (s == 5) return "SIGTRAP"; if (s == 6) return "SIGABRT"
    if (s == 7) return "SIGBUS"; if (s == 9) return "SIGKILL"; if (s == 11) return "SIGSEGV"
    return "signal " s
  }
  {
    k = $1 FS $2 FS $3
    if (!(k in count)) { order[++groups] = k }
    count[k]++
    names[k] = names[k] "\n    " $4
  }
  END {
    # The largest group first (a selection sort; groups are few).
    for (i = 1; i <= groups; i++) for (j = i + 1; j <= groups; j++)
      if (count[order[j]] > count[order[i]]) { t = order[i]; order[i] = order[j]; order[j] = t }
    for (i = 1; i <= groups; i++) {
      k = order[i]; split(k, p, FS)
      tag = signame(p[1])
      if (p[1] == 9) tag = tag " (killed, probably out of memory)"
      else if (p[2] == "") tag = tag " (no panic text: replay under gdb)"
      printf "%d file(s)  %s\n  panic:  %s\n  frames: %s\n  files:%s\n\n", count[k], tag, \
        (p[2] == "" ? "-" : p[2]), (p[3] == "" ? "-" : p[3]), names[k]
    }
    printf "triage.sh: %d file(s), %d group(s)\n", NR, groups
  }'
