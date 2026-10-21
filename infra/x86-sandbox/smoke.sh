#!/usr/bin/env bash
# Smoke test for ep9-x86-sandbox. Raw logs go to evidence/smoke/ (git-ignored).
# usage: ./smoke.sh [all|health|info|tools|rt|bg|auth|destroy]   (env SBX_ID, default ep9-smoke)
# `all` ends with `destroy`, so no container is left running (it bills while it runs).
# Never prints the token: curl reads it from a process-substitution header file.
set -uo pipefail
H=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
E="$H/evidence/smoke"
SBX="${SBX_CLIENT:-$H/sbx}"
URL=$(tr -d ' \n\r' < "$H/.url")
URL=${URL%/}
# The token only travels over HTTPS (same rule as sbx).
case "$URL" in https://*) ;; *) echo "smoke: refusing non-https URL '$URL'" >&2; exit 99 ;; esac
ID=${SBX_ID:-ep9-smoke}
mkdir -p "$E"
step="${1:-all}"

log() { printf '\n===== %s (%s)\n' "$*" "$(date -u +%FT%TZ)"; }

run_health() {
	log "health (no id, then id $ID; never starts a container)"
	"$SBX" health; echo "exit=$?"
	"$SBX" health "$ID"; echo "exit=$?"
}

run_info() {
	log "system info"
	"$SBX" exec "$ID" '
echo "--- uname -a:"; uname -a
echo "--- nproc: $(nproc)"
echo "--- free -g:"; free -g
echo "--- df -h /:"; df -h /
echo "--- model:"; grep -m1 "model name" /proc/cpuinfo
echo "--- flags (of fma avx2 avx512f):"
for f in fma avx2 avx512f; do if grep -m1 "^flags" /proc/cpuinfo | tr " " "\n" | grep -qx "$f"; then echo "$f yes"; else echo "$f NO"; fi; done
echo "--- os:"; grep -E "^PRETTY_NAME=" /etc/os-release; ldd --version | head -1
' 300
	echo "exit=$?"
}

run_tools() {
	log "tool versions"
	"$SBX" exec "$ID" 'for c in "valgrind --version" "node --version" "perf --version" "gdb --version" "strace -V" "rustc --version" "cargo --version" "gcc --version" "git --version" "zstd --version" "jq --version" "python3 --version"; do printf "%-18s %s\n" "$c:" "$($c 2>&1 | head -1)"; done; ls -la /usr/local/bin/sandbox-shim' 120
	echo "exit=$?"
	log "tools work: cachegrind, perf stat, gdb, strace on /bin/true"
	"$SBX" exec "$ID" 'set -e
cd /tmp
valgrind --tool=cachegrind --cache-sim=no --cachegrind-out-file=/tmp/cg.out /bin/true 2>&1 | tail -3
ls -la /tmp/cg.out
echo "--- perf_event_paranoid: $(cat /proc/sys/kernel/perf_event_paranoid)"
perf stat -e instructions,cycles -- /bin/true 2>&1 | grep -E "instructions|cycles|not supported" || true
gdb -batch -ex "info functions ^main$" /bin/true 2>&1 | tail -2
strace -c -o /tmp/st.txt /bin/true && head -3 /tmp/st.txt' 120
	echo "exit=$?"
}

run_rt() {
	log "20 MB random file round trip (put + get, sha256 match)"
	local f="$E/rt-20M.bin" back="$E/rt-20M.back.bin" t0 t1 t2
	head -c 20971520 /dev/urandom > "$f"
	echo "local sha256: $(shasum -a 256 < "$f" | cut -d' ' -f1)  size: $(wc -c < "$f" | tr -d ' ')"
	t0=$(date +%s)
	"$SBX" put "$ID" "$f" /work/rt/rt-20M.bin; echo "put exit=$?"
	t1=$(date +%s)
	"$SBX" get "$ID" /work/rt/rt-20M.bin "$back"; echo "get exit=$?"
	t2=$(date +%s)
	echo "put seconds: $((t1 - t0))  get seconds: $((t2 - t1))"
	echo "back sha256:  $(shasum -a 256 < "$back" | cut -d' ' -f1)"
	cmp "$f" "$back" && echo "cmp: IDENTICAL"
	"$SBX" exec "$ID" 'ls -la /work/rt; sha256sum < /work/rt/rt-20M.bin' 60
	rm -f "$f" "$back"
}

run_bg() {
	log "bg + wait: a 200 s job (one line every 20 s, then exit 0)"
	local s; s=$(date +%s)
	"$SBX" bg "$ID" smoke200 'for i in $(seq 1 10); do echo "tick $i $(date -u +%T)"; sleep 20; done; echo done'
	echo "bg exit=$?"
	"$SBX" wait "$ID" smoke200 30; echo "wait exit=$? wall=$(( $(date +%s) - s ))s"
}

# code METHOD URL AUTHMODE -> HTTP status (no right-token mode here on purpose)
code() {
	local args=(-sS -o /dev/null -w '%{http_code}' -X "$1" --max-time 60)
	case "$3" in
		none) curl "${args[@]}" "$2" ;;
		wrong) curl "${args[@]}" -H 'Authorization: Bearer 0000000000000000000000000000000000000000000000000000000000000000' "$2" ;;
	esac
}

run_auth() {
	log "auth negative checks (expect no token 401, wrong token 401, http:// refused)"
	local d='{"id":"ep9-authprobe"}'
	for r in "POST /exec" "POST /write" "POST /destroy" "GET /health"; do
		set -- $r
		printf '%-5s %-9s none=%s wrong=%s\n' "$1" "$2" "$(code "$1" "$URL$2" none)" "$(code "$1" "$URL$2" wrong)"
	done
	echo "--- unauthenticated /exec with a body: $(curl -sS -o /dev/null -w '%{http_code}' --max-time 60 -H 'content-type: application/json' --data-binary '{"id":"ep9-authprobe","cmd":"touch /tmp/pwned"}' "$URL/exec")"
	local http="http://${URL#https://}"
	echo "--- plain http, no token sent (Worker must answer 403): GET /health -> $(code GET "$http/health" none), POST /exec -> $(code POST "$http/exec" none)"
	printf '%s\n' "$http" > "$E/http.url"
	SBX_URL_FILE="$E/http.url" "$SBX" health > "$E/sbx-http.out" 2>&1
	echo "--- sbx with an http:// URL file: exit=$? (expect 99, refused before curl): $(head -c 160 "$E/sbx-http.out")"
	rm -f "$E/http.url" "$E/sbx-http.out"
	echo "--- the probe id was never started:"
	"$SBX" health ep9-authprobe
}

run_destroy() {
	log "destroy $ID, then health (expect running:false, registered:false)"
	"$SBX" destroy "$ID"; echo "destroy exit=$?"
	"$SBX" health "$ID"; echo "health exit=$?"
}

case "$step" in
	all) run_health; run_info; run_tools; run_rt; run_bg; run_auth; run_destroy ;;
	*) "run_$step" ;;
esac
