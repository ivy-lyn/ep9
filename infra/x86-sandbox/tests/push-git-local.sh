#!/usr/bin/env bash
# push-git-local.sh - tests the local parts of `sbx push-git` and `sbx manifest` with temp
# git repos. No sandbox, no network, no token:
#   - SBX_URL_FILE and SBX_TOKEN_FILE point at files that do not exist, so any Worker call
#     would stop with "missing URL file" before it sends anything;
#   - the end-to-end cases source sbx and replace the two Worker calls (cmd_put, rexec) with
#     local stand-ins that copy the archive and run the box-side script with bash here.
#
# Usage:  bash infra/x86-sandbox/tests/push-git-local.sh
#         SBX_TEST_BASH=/bin/bash bash infra/x86-sandbox/tests/push-git-local.sh   (sbx under bash 3.2)
# Needs:  git (with `git archive --add-file`), zstd, jq, tar. Exit 0 = all cases pass.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SBX=$HERE/../sbx
BASH_BIN=${SBX_TEST_BASH:-bash}
T=$(mktemp -d "${TMPDIR:-/tmp}/sbx-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
T=$(cd "$T" && pwd -P)

export SBX_URL_FILE="$T/no-such.url" SBX_TOKEN_FILE="$T/no-such.token"
# Keep the user's git config out: no signing, no hooks, no attributes file.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=sbx-test GIT_AUTHOR_EMAIL=sbx-test GIT_COMMITTER_NAME=sbx-test GIT_COMMITTER_EMAIL=sbx-test

for t in git zstd jq tar; do command -v "$t" >/dev/null || { echo "push-git-local: need $t" >&2; exit 2; }; done

PASS=0 FAIL=0
ok() { PASS=$((PASS + 1)); echo "ok   $*"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL $*"; }
check() { if [ "$1" = 0 ]; then ok "$2"; else bad "$2"; fi; }

# sbx CLI: RC, $T/out, $T/err
RC=0
sbx() { RC=0; "$BASH_BIN" "$SBX" "$@" > "$T/out" 2> "$T/err" || RC=$?; }
# expect RC TEXT DESC: exit code RC and TEXT in stderr
expect() {
	if [ "$RC" = "$1" ] && grep -qF -- "$2" "$T/err"; then ok "$3"; else
		bad "$3 (want exit $1 and '$2' in stderr; got exit $RC)"; sed 's/^/     | /' "$T/err" | head -5
	fi
}
# A local refusal must come before the URL / token step: nothing could have been sent.
local_only() {
	if grep -q 'missing URL file\|missing token file' "$T/err"; then bad "$1 (reached the remote step)"; else ok "$1: refused before any remote step"; fi
}

commit() { git -C "$1" -c core.safecrlf=false add -A && git -C "$1" -c commit.gpgsign=false commit -q -m "$2"; }

# The main test repo: spaces, byte order (B < a < 'a b' < a-b < a.txt < a/), an executable,
# an empty file, a binary file, a symlink, a gitlink, and the repo's own eol=lf rule.
R=$T/repo
git init -q "$R"
printf '* text=auto eol=lf\n' > "$R/.gitattributes"
printf 'B\n' > "$R/B.txt"
printf 'a\n' > "$R/a.txt"
mkdir -p "$R/a" "$R/a b" "$R/dir/sub"
printf 'ab\n' > "$R/a/b.txt"
printf 'space\n' > "$R/a b/c d.txt"
printf 'dash\n' > "$R/a-b"
printf 'fn main() {}\n' > "$R/dir/sub/deep.rs"
printf '#!/bin/sh\necho hi\n' > "$R/run.sh"; chmod +x "$R/run.sh"
: > "$R/empty"
printf '\000\001\002\377binary\000' > "$R/bin.dat"
printf 'old\n' > "$R/gone-in-2.txt"
ln -s a.txt "$R/link"
commit "$R" one
# A gitlink (submodule entry): git archive writes an empty dir and no file for it.
git -C "$R" update-index --add --cacheinfo "160000,$(git -C "$R" rev-parse HEAD),sub"
git -C "$R" -c commit.gpgsign=false commit -q -m "gitlink"
C1=$(git -C "$R" rev-parse HEAD)
rm "$R/gone-in-2.txt"; printf 'new\n' > "$R/new-in-2.txt"; printf 'B2\n' > "$R/B.txt"
commit "$R" two
C2=$(git -C "$R" rev-parse HEAD)

# ---- 1. sbx manifest (local only) ----
sbx manifest "$R" "$C1"
check "$RC" "manifest: exit 0 with no URL and no token files"
cp "$T/out" "$T/m1"
if [ "$(sed -n 1p "$T/m1")" = "commit $C1" ]; then ok "manifest: line 1 is 'commit <full id>'"; else bad "manifest: line 1"; fi
{
	printf 'commit %s\n' "$C1"
	for p in '.gitattributes' 'B.txt' 'a b/c d.txt' 'a-b' 'a.txt' 'a/b.txt' 'bin.dat' 'dir/sub/deep.rs' 'empty' 'gone-in-2.txt' 'link' 'run.sh'; do
		printf '%s %s\n' "$(git -C "$R" rev-parse "$C1:$p")" "$p"
	done
} > "$T/m1.want"
cmp -s "$T/m1" "$T/m1.want"; check $? "manifest: one '<blob id> <path>' line per file, byte order, gitlink skipped"
cmp -s "$T/m1" "$T/m1.want" || diff "$T/m1.want" "$T/m1" | sed 's/^/     | /'
sbx manifest "$R" HEAD~1
cmp -s "$T/out" "$T/m1"; check $? "manifest: a rev name gives the same manifest as the full id"
sbx manifest "$R" no-such-rev
expect 99 "unknown commit" "manifest: unknown commit"
sbx manifest "$R"
expect 99 "usage: sbx manifest" "manifest: wrong arg count"

# A commit that holds its own .sbx-manifest or a path with a newline cannot be described.
R2=$T/repo-own
git init -q "$R2"; printf 'x\n' > "$R2/.sbx-manifest"; commit "$R2" own
sbx manifest "$R2" HEAD
expect 92 "has its own .sbx-manifest" "manifest: refuses a commit with its own .sbx-manifest"
R3=$T/repo-nl
git init -q "$R3"; printf 'x\n' > "$R3/bad
name"; commit "$R3" nl
sbx manifest "$R3" HEAD
expect 92 "path with a newline" "manifest: refuses a path with a newline"

# ---- 2. push-git argument checks (CLI; all before any remote step) ----
for c in \
	"99|usage: sbx push-git|3 args|sbx-t $R $C1" \
	"99|bad id|bad id|Bad_Id $R $C1 /work/src" \
	"99|must be absolute|relative remote dir|sbx-t $R $C1 work/src" \
	"99|must not be /|remote dir /|sbx-t $R $C1 /" \
	"99|holds the upload dir|remote dir /work|sbx-t $R $C1 /work" \
	"99|holds the upload dir|remote dir = upload dir|sbx-t $R $C1 /work/.upload" \
	"99|plain path|trailing slash|sbx-t $R $C1 /work/src/" \
	"99|plain path|dot component|sbx-t $R $C1 /work/./src" \
	"99|plain path|dot-dot component|sbx-t $R $C1 /work/../src" \
	"99|plain path|double slash|sbx-t $R $C1 /work//src" \
	"99|unknown option|unknown option|--wipe sbx-t $R $C1 /work/src" \
	"99|unknown commit|unknown commit|sbx-t $R no-such-rev /work/src"; do
	IFS='|' read -r want text desc args <<< "$c"
	# shellcheck disable=SC2086 # the args have no spaces (the temp path has none, checked below)
	sbx push-git $args
	expect "$want" "$text" "push-git: $desc"
	local_only "push-git: $desc"
done
case "$R" in *' '*) bad "the temp dir has a space; the arg cases above split on spaces" ;; esac

# A good call with no URL file: the archive and manifest are built and checked, then the
# remote step stops at the missing URL file. Nothing is sent.
sbx push-git sbx-t "$R" "$C1" /work/src
expect 99 "missing URL file" "push-git: good args reach the remote step only after the local build"
sbx push-git --clear -- sbx-t "$R" "$C1" /work/src
expect 99 "missing URL file" "push-git: --clear and -- are accepted"

# ---- 3. the archive self-check: archive bytes must equal the blobs the manifest names ----
mkattr() { # DIR ATTR-LINE FILE CONTENT
	git init -q "$1"; printf '%s\n' "$2" > "$1/.gitattributes"; printf '%s\n' "$4" > "$1/$3"; printf 'plain\n' > "$1/plain.txt"; commit "$1" attr
}
mkattr "$T/repo-crlf" 'crlf.txt text eol=crlf' crlf.txt 'line'
sbx push-git sbx-t "$T/repo-crlf" HEAD /work/src
expect 92 "archive bytes differ" "push-git: eol=crlf changes the archive bytes: refused"
local_only "push-git: eol=crlf"
# shellcheck disable=SC2016 # a literal git export-subst placeholder
mkattr "$T/repo-subst" 'subst.txt export-subst' subst.txt '$Format:%H$'
sbx push-git sbx-t "$T/repo-subst" HEAD /work/src
expect 92 "archive bytes differ" "push-git: export-subst changes the archive bytes: refused"
mkattr "$T/repo-ignore" 'skip.txt export-ignore' skip.txt 'skip'
sbx push-git sbx-t "$T/repo-ignore" HEAD /work/src
expect 92 "export-ignore" "push-git: export-ignore drops a file: refused"
sbx push-git sbx-t "$R2" HEAD /work/src
expect 92 "has its own .sbx-manifest" "push-git: a commit with its own .sbx-manifest: refused"

# ---- 4. end to end with local stand-ins for the Worker ----
# The box is $T/box; uploads go to $T/box/.upload (UPLOAD_DIR); the box-side script runs with
# bash here. Usage of fake.sh: same args as `sbx push-git`.
BOX=$T/box
mkdir -p "$BOX"
cat > "$T/fake.sh" <<EOF
. $(printf '%q' "$SBX")
UPLOAD_DIR=$(printf '%q' "$BOX/.upload")
sbx_init_remote() { :; }
cmd_put() { mkdir -p "\$(dirname "\$3")"; cp "\$2" "\$3"; }
rexec() {  # ID CMD TIMEOUT OUT [IDEM]: run CMD here, write the Worker's JSON shape
	local ec=0
	bash -c "\$2" > "\$4.o" 2> "\$4.e" || ec=\$?
	jq -n --rawfile o "\$4.o" --rawfile e "\$4.e" --argjson ec "\$ec" '{exitCode:\$ec, stdout:\$o, stderr:\$e}' > "\$4"
}
sbx_make_work
cmd_push_git "\$@"
EOF
fake() { RC=0; "$BASH_BIN" "$T/fake.sh" "$@" > "$T/out" 2> "$T/err" || RC=$?; }

# verify_tree DIR COMMIT DESC: a reader's check written apart from sbx. The tree holds exactly
# the manifest's files plus .sbx-manifest and .sbx-commit, and each file hashes to its blob id.
verify_tree() {
	local d=$1 c=$2 desc=$3 line sha p got n want e=0
	[ "$(cat "$d/.sbx-commit" 2>/dev/null)" = "$c" ] || { bad "$desc: .sbx-commit"; e=1; }
	[ "$(sed -n 1p "$d/.sbx-manifest")" = "commit $c" ] || { bad "$desc: manifest line 1"; e=1; }
	sbx manifest "$R" "$c"
	cmp -s "$T/out" "$d/.sbx-manifest" || { bad "$desc: .sbx-manifest differs from 'sbx manifest'"; e=1; }
	while IFS= read -r line; do
		sha=${line%% *}; p=${line#* }
		if [ -L "$d/$p" ]; then got=$(printf '%s' "$(readlink "$d/$p")" | git hash-object --stdin)
		else got=$(git hash-object --no-filters -- "$d/$p" 2>/dev/null); fi
		[ "$got" = "$sha" ] || { bad "$desc: $p hashes to '$got', manifest says $sha"; e=1; }
	done < <(sed 1d "$d/.sbx-manifest")
	n=$(find "$d" \( -type f -o -type l \) | wc -l | tr -d ' ')
	want=$(($(wc -l < "$d/.sbx-manifest") - 1 + 2))
	[ "$n" = "$want" ] || { bad "$desc: $n files in the tree, want $want"; e=1; }
	[ -x "$d/run.sh" ] || { bad "$desc: run.sh lost its x bit"; e=1; }
	[ "$e" = 0 ] && ok "$desc: tree matches the manifest (every blob id, file count, .sbx-commit)"
}
no_upload() { if [ -n "$(ls -A "$BOX/.upload" 2>/dev/null)" ]; then bad "$1: upload left behind"; else ok "$1: upload removed"; fi; }
listing() { (cd "$1" && find . -print | LC_ALL=C sort && cat .sbx-commit 2>/dev/null); }

fake sbx-t "$R" "$C1" "$BOX/src"
check "$RC" "e2e: push into a new dir: exit 0"
grep -q '^extracted 14 files$' "$T/out"; check $? "e2e: box reports 14 files (12 incl. the symlink + .sbx-manifest + .sbx-commit)"
verify_tree "$BOX/src" "$C1" "e2e new dir"
no_upload "e2e new dir"

listing "$BOX/src" > "$T/before"
fake sbx-t "$R" "$C2" "$BOX/src"
expect 93 "is not empty" "e2e: push into a non-empty dir: exit 93"
grep -qF -- "--clear" "$T/err"; check $? "e2e: the refusal names --clear"
listing "$BOX/src" > "$T/after"
cmp -s "$T/before" "$T/after"; check $? "e2e: a refused push leaves the dir as it was"
no_upload "e2e refused"

printf 'stale\n' > "$BOX/src/stale.txt"; printf 'h\n' > "$BOX/src/.hidden"; mkdir -p "$BOX/src/target/debug"
fake --clear sbx-t "$R" "$C2" "$BOX/src"
check "$RC" "e2e: --clear into a non-empty dir: exit 0"
if [ ! -e "$BOX/src/gone-in-2.txt" ] && [ ! -e "$BOX/src/stale.txt" ] && [ ! -e "$BOX/src/.hidden" ] && [ ! -e "$BOX/src/target" ]; then
	ok "e2e: --clear removed the old commit's file, a stale file, a dotfile and a dir"
else bad "e2e: --clear left an old file"; fi
verify_tree "$BOX/src" "$C2" "e2e --clear"
no_upload "e2e --clear"

mkdir -p "$BOX/only-dot"; : > "$BOX/only-dot/.x"
fake sbx-t "$R" "$C1" "$BOX/only-dot"
expect 93 "is not empty" "e2e: a dir with only a dotfile is not empty"

mkdir -p "$BOX/empty-dir"
fake sbx-t "$R" "$C1" "$BOX/empty-dir"
check "$RC" "e2e: an existing empty dir is accepted"
verify_tree "$BOX/empty-dir" "$C1" "e2e empty dir"

printf 'f\n' > "$BOX/a-file"
fake --clear sbx-t "$R" "$C1" "$BOX/a-file"
expect 93 "not a directory" "e2e: a file at the target is refused, also with --clear"
if [ "$(cat "$BOX/a-file")" = f ]; then ok "e2e: the file at the target is untouched"; else bad "e2e: the file at the target changed"; fi

mkdir -p "$BOX/real"; ln -s "$BOX/real" "$BOX/sym"
fake sbx-t "$R" "$C1" "$BOX/sym"
expect 93 "is a symlink" "e2e: a symlink at the target is refused"
no_upload "e2e symlink"

fake --clear sbx-t "$R" "$C1" "$BOX"
expect 99 "holds the upload dir" "e2e: the parent of the upload dir is refused before the build"

# An ancestor symlink: $T/alias -> $T, so $T/alias/box passes the local (lexical) checks but is
# $BOX, the parent of the upload dir. The box resolves the path first and refuses it; nothing
# under $BOX is deleted.
ln -s "$T" "$T/alias"
mkdir -p "$BOX/keep"; printf 'k\n' > "$BOX/keep/f"
listing "$BOX" > "$T/before"
fake --clear sbx-t "$R" "$C1" "$T/alias/box"
expect 93 "holds the upload dir" "e2e: --clear through an ancestor symlink to the upload dir's parent is refused on the box"
listing "$BOX" | grep -v '^\./\.upload' > "$T/after"; grep -v '^\./\.upload' "$T/before" > "$T/before.k"
cmp -s "$T/before.k" "$T/after"; check $? "e2e: the ancestor-symlink refusal deleted nothing"
no_upload "e2e ancestor symlink"
# Through the same alias to a dir of its own: the resolved dir is cleared and filled.
mkdir -p "$BOX/src"; printf 'stale\n' > "$BOX/src/stale.txt"
fake --clear sbx-t "$R" "$C1" "$T/alias/box/src"
check "$RC" "e2e: --clear through an ancestor symlink to a plain dir: exit 0"
if [ -e "$BOX/src/stale.txt" ]; then bad "e2e: --clear through an ancestor symlink left a stale file"; else ok "e2e: --clear through an ancestor symlink cleared the resolved dir"; fi
verify_tree "$BOX/src" "$C1" "e2e ancestor symlink"

# ---- 5. lint ----
bash -n "$SBX"; check $? "bash -n sbx"
if [ -x /bin/bash ]; then /bin/bash -n "$SBX"; check $? "/bin/bash -n sbx"; fi
if command -v shellcheck >/dev/null; then
	shellcheck -S warning "$SBX" "${BASH_SOURCE[0]}"; check $? "shellcheck -S warning sbx and this test"
else
	echo "skip shellcheck (not installed)"
fi

# shellcheck disable=SC2016 # expands in the child bash
echo "push-git-local: $PASS passed, $FAIL failed (bash: $("$BASH_BIN" -c 'echo $BASH_VERSION'))"
[ "$FAIL" = 0 ]
