#!/usr/bin/env bash
# Tests scripts/sync-lib.sh (sync_dep) with temp git repos: no network, no real pin.
#   - a stamp is accepted only when HEAD is the pin and the files are the pin plus the patches;
#     an edited file, an extra file or another HEAD makes sync_dep sync again;
#   - an ignored file (a build dir such as target/ or Fuzzilli's .build) does not;
#   - the stamp is gone while sync_dep changes the checkout (a failed fetch leaves no stamp);
#   - fork_attest (FORK-CHECKOUT-ATTEST) reads "pinned" only for a checkout that sync_dep would
#     keep, under sh and dash too: a checkout on an old patch set, an edited file, an extra file,
#     another HEAD or no stamp reads "stale", and no checkout "absent" (the negative controls).
# Usage: bash scripts/tests/sync-lib-test.sh   Exit 0 = all cases pass.
set -uo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
LIB=$HERE/../sync-lib.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/sync-lib-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
T=$(cd "$T" && pwd -P)
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t

PASS=0 FAIL=0
ok() { PASS=$((PASS + 1)); echo "ok   $*"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL $*"; }
check() { if [ "$1" = 0 ]; then ok "$2"; else bad "$2"; fi; }

# The upstream repo: two commits; target/ is ignored.
U=$T/upstream
git init -q "$U"
git -C "$U" config uploadpack.allowAnySHA1InWant true
mkdir -p "$U/src"
printf 'target/\n' > "$U/.gitignore"
printf 'one\n' > "$U/a.txt"
printf 'fn f() {}\n' > "$U/src/lib.rs"
git -C "$U" add -A && git -C "$U" commit -q -m one
C1=$(git -C "$U" rev-parse HEAD)
printf 'fn g() {}\n' > "$U/src/lib.rs"
git -C "$U" add -A && git -C "$U" commit -q -m two
C2=$(git -C "$U" rev-parse HEAD)

# The repo root: pin C1, one patch (edits a.txt, adds new.txt).
R=$T/root
mkdir -p "$R/third_party/patches/dep"
printf '%s\n' "$C1" > "$R/third_party/dep.commit"
cat > "$R/third_party/patches/dep/0001-p.patch" <<'EOF'
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-one
+one patched
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+new
EOF
D=$R/third_party/dep

# sync: runs sync_dep in a child bash as the sync scripts do; RC and $T/err.
sync() {
	RC=0
	bash -c 'set -euo pipefail; root=$1; . "$2"; sync_dep dep "$3"' _ "$R" "$LIB" "file://$U" \
		> /dev/null 2> "$T/err" || RC=$?
}
# offline / online: the upstream is unreachable / back (an accepted stamp fetches nothing).
offline() { mv "$U" "$U.away"; }
online() { mv "$U.away" "$U"; }
pinned() { # DESC: HEAD is C1 and the files are C1 plus the patch, with no extra file
	local e=0
	[ "$(git -C "$D" rev-parse HEAD)" = "$C1" ] || e=1
	[ "$(cat "$D/a.txt")" = "one patched" ] && [ "$(cat "$D/new.txt")" = new ] || e=1
	[ "$(cat "$D/src/lib.rs")" = 'fn f() {}' ] || e=1
	[ ! -e "$D/src/extra.rs" ] || e=1
	check "$e" "$1"
}

sync
check "$RC" "first sync: exit 0"
pinned "first sync: HEAD is the pin, the patch is applied"
[ -f "$D/.ep9-stamp" ]; check $? "first sync: stamp written"

offline
sync
check "$RC" "an unchanged checkout: the stamp is accepted with no fetch"
mkdir -p "$D/target"; printf 'x\n' > "$D/target/build.o"
sync
check "$RC" "an ignored build file does not reject the stamp (no fetch)"
online

printf 'fn evil() {}\n' > "$D/src/lib.rs"
sync
check "$RC" "an edited tracked file: exit 0"
pinned "an edited tracked file is put back to the pin"

printf 'two\n' > "$D/a.txt"
sync
pinned "an edited patched file is put back to the pin plus the patch"

printf 'fn extra() {}\n' > "$D/src/extra.rs"
sync
pinned "an extra untracked file is removed"

printf 'fn staged() {}\n' > "$D/src/lib.rs"; git -C "$D" add src/lib.rs
sync
pinned "a staged edit is put back to the pin"

git -C "$D" fetch -q --depth 1 origin "$C2" && git -C "$D" checkout -q -f --detach "$C2"
printf 'one patched\n' > "$D/a.txt"   # new.txt (untracked) stays from the pin's sync
sync
pinned "another HEAD (with the patched files on top) is moved back to the pin"
[ -d "$D/target" ]; check $? "an ignored dir stays through a sync (git clean -fd keeps it)"

# The stamp goes before the checkout changes: HEAD is wrong and the fetch fails.
git -C "$D" fetch -q --depth 1 origin "$C2" && git -C "$D" checkout -q -f --detach "$C2"
offline
sync
[ "$RC" != 0 ]; check $? "another HEAD and no upstream: sync fails (the stamp is not accepted)"
[ ! -e "$D/.ep9-stamp" ]; check $? "a failed sync leaves no stamp"
online
sync
check "$RC" "the next sync with the upstream back: exit 0"
pinned "the next sync: HEAD is the pin"

# attest SH NAMES...: fork_attest in a child shell SH, as the build scripts run it; OUT, RC.
attest() {
	local sh=$1
	shift
	RC=0
	OUT=$("$sh" -c '. "$1"; root=$2; shift 2; fork_attest "$@"' _ "$LIB" "$R" "$@") || RC=$?
}
# want SH LINE RC DESC
want() {
	attest "$1" dep
	local e=1
	if [ "$OUT" = "$2" ] && [ "$RC" = "$3" ]; then e=0; fi
	check "$e" "$4 ($1: $OUT, status $RC)"
}
SHELLS="bash sh"
if command -v dash >/dev/null; then SHELLS="$SHELLS dash"; fi
for s in $SHELLS; do
	sync
	want "$s" "dep=pinned" 0 "attest: a synced checkout is pinned"
	mkdir -p "$D/target"; printf 'x\n' > "$D/target/build.o"
	want "$s" "dep=pinned" 0 "attest: an ignored build file keeps it pinned"
	attest "$s" dep nope
	e=1; if [ "$OUT" = "dep=pinned nope=absent" ] && [ "$RC" = 1 ]; then e=0; fi
	check "$e" "attest: no checkout is absent, status 1 ($s: $OUT)"
	printf 'fn evil() {}\n' > "$D/src/lib.rs"
	want "$s" "dep=stale" 1 "attest: an edited tracked file is stale"
	sync
	printf 'one\n' > "$D/a.txt"
	want "$s" "dep=stale" 1 "attest: a patched file put back to the pin is stale"
	sync
	printf 'fn extra() {}\n' > "$D/src/extra.rs"
	want "$s" "dep=stale" 1 "attest: an extra untracked file is stale"
	sync
	git -C "$D" fetch -q --depth 1 origin "$C2" && git -C "$D" checkout -q -f --detach "$C2"
	want "$s" "dep=stale" 1 "attest: another HEAD is stale"
	sync
	cp "$D/.ep9-stamp" "$T/stamp"; rm -f "$D/.ep9-stamp"
	want "$s" "dep=stale" 1 "attest: no stamp is stale"
	cp "$T/stamp" "$D/.ep9-stamp"
	want "$s" "dep=pinned" 0 "attest: the stamp back is pinned again"
	# The negative control of the route: the checkout of an old patch set. A second patch joins
	# the set; the checkout still has only the first one (a sync before the patch landed).
	cp "$R/third_party/patches/dep/0001-p.patch" "$T/0001-p.patch"
	cat > "$R/third_party/patches/dep/0002-q.patch" <<'EOF2'
diff --git a/new.txt b/new.txt
--- a/new.txt
+++ b/new.txt
@@ -1 +1 @@
-new
+new and q
EOF2
	want "$s" "dep=stale" 1 "attest: a checkout on an old patch set is stale"
	sync
	want "$s" "dep=pinned" 0 "attest: the sync of the new patch set is pinned"
	[ "$(cat "$D/new.txt")" = "new and q" ]; check $? "attest: the new patch set is applied"
	rm -f "$R/third_party/patches/dep/0002-q.patch"
	want "$s" "dep=stale" 1 "attest: a checkout that has a patch the set lost is stale"
	sync
	e=0; for f in "$D"/.git/ep9-sync-index*; do [ -e "$f" ] && e=1; done
	check "$e" "attest: no scratch index is left ($s)"
done

if command -v shellcheck >/dev/null; then
	shellcheck -S warning "$LIB" "${BASH_SOURCE[0]}"; check $? "shellcheck -S warning sync-lib.sh and this test"
else
	echo "skip shellcheck (not installed)"
fi
echo "sync-lib-test: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
