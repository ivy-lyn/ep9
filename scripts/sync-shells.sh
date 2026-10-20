#!/usr/bin/env bash
# Installs the reference shells pinned in third_party/shells.toml into third_party/shells/ (git-ignored).
# usage: bash scripts/sync-shells.sh [node] [d8] [quickjs] [bun] [brimstone]   (default: all)
# A second run with the same pins does nothing. Each install dir has a .ep9-stamp with its pin.
# The end of the run prints each shell's version and path, and writes third_party/shells/versions.txt.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
toml="$root/third_party/shells.toml"
out="$root/third_party/shells"

# pin <section> <key>: the string value of `key = "value"` in [section]; empty if absent.
pin() {
  awk -v s="[$1]" -v k="$2" '
    /^\[/ { in_s = ($0 == s); next }
    in_s && $1 == k && $2 == "=" { v = $0; sub(/^[^"]*"/, "", v); sub(/".*$/, "", v); print v; exit }
  ' "$toml"
}
need() { local v; v="$(pin "$1" "$2")"; [ -n "$v" ] || { echo "sync-shells: no $2 in [$1] of $toml" >&2; exit 1; }; echo "$v"; }

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) plat=darwin_arm64 ;;
  Linux-x86_64) plat=linux_x64 ;;
  Linux-aarch64 | Linux-arm64) plat=linux_arm64 ;;
  MINGW*-x86_64 | MSYS*-x86_64 | CYGWIN*-x86_64) plat=windows_x64 ;;
  *) echo "sync-shells: unsupported host $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
exe=""; [ "$plat" = windows_x64 ] && exe=".exe"

sha256() { if command -v sha256sum > /dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
check_sha() { # check_sha <file> <want>
  local got; got="$(sha256 "$1")"
  [ "$got" = "$2" ] || { echo "sync-shells: sha256 mismatch for $1: got $got, want $2" >&2; exit 1; }
}
fresh() { [ "$(cat "$1/.ep9-stamp" 2>/dev/null || true)" = "$2" ]; } # fresh <dir> <stamp>
unpack() { # unpack <archive> <dir>
  case "$1" in
    *.zip) if command -v unzip > /dev/null; then unzip -q -o "$1" -d "$2"; else tar -xf "$1" -C "$2"; fi ;;
    *) tar -xzf "$1" -C "$2" ;;
  esac
}
git_at() { # git_at <dir> <url> <commit>: shallow checkout of one commit
  [ -d "$1/.git" ] || { git init -q "$1"; git -C "$1" remote add origin "$2"; }
  git -C "$1" fetch -q --depth 1 origin "$3"
  git -C "$1" checkout -q -f --detach FETCH_HEAD
}
# fetch_asset <section> <dir>: download the platform asset, check it, unpack it into <dir>.
fetch_asset() {
  local asset sum tmp
  asset="$(need "$1" "asset_$plat")"; sum="$(need "$1" "sha256_$plat")"
  rm -rf "$2"; mkdir -p "$2"; tmp="$2/$asset"
  curl -fsSL --retry 3 -o "$tmp" "$(need "$1" base_url)$asset"
  check_sha "$tmp" "$sum"
  unpack "$tmp" "$2"; rm -f "$tmp"
}
has_asset() { [ -n "$(pin "$1" "asset_$plat")" ] || { echo "$1: no build for $plat (skipped)"; return 1; }; }

node_bin=""; d8_bin=""; qjs_bin=""; bun_bin=""; bs_bin=""

sync_node() {
  local v d stamp; v="$(need node version)"; d="$out/node-$v"; has_asset node || return 0
  stamp="$v $(need node "sha256_$plat")"
  if ! fresh "$d" "$stamp"; then fetch_asset node "$d"; echo "$stamp" > "$d/.ep9-stamp"; fi
  if [ "$plat" = windows_x64 ]; then node_bin="$(echo "$d"/node-v*/node.exe)"; else node_bin="$(echo "$d"/node-v*/bin/node)"; fi
  # [P1]: node is built without pointer compression; 32-bit Smis show it at run time.
  [ "$("$node_bin" -p "process.config.variables.v8_enable_pointer_compression ? 'true' : 'false'")" = "$(need node pointer_compression)" ] ||
    { echo "sync-shells: node pointer compression differs from the pin" >&2; exit 1; }
}

sync_d8() {
  local v jv jos d stamp; v="$(need d8 version)"; jv="$(need d8 jsvu_version)"; d="$out/d8-$v"
  jos="$(pin d8 "jsvu_os_$plat")"; [ -n "$jos" ] || { echo "d8: jsvu has no V8 build for $plat (skipped)"; return 0; }
  stamp="$v $jv $(need d8 "sha256_$plat")"
  if ! fresh "$d" "$stamp"; then
    rm -rf "$d"; mkdir -p "$d/home"
    npm install --silent --no-save --no-audit --no-fund --prefix "$d/jsvu" "jsvu@$jv" > /dev/null
    # jsvu installs into ~/.jsvu; HOME points it at our dir.
    HOME="$d/home" USERPROFILE="$d/home" node "$d/jsvu/node_modules/jsvu/cli.js" --os="$jos" "v8@$v" > "$d/jsvu.log" 2>&1 ||
      { cat "$d/jsvu.log" >&2; exit 1; }
    check_sha "$d/home/.jsvu/engines/v8-$v/v8-$v$exe" "$(need d8 "sha256_$plat")"
    echo "$stamp" > "$d/.ep9-stamp"
  fi
  d8_bin="$d/home/.jsvu/engines/v8-$v/v8-$v$exe"
  # Pointer compression gives 31-bit Smis: 2**30 is then not a Smi.
  local smi; smi="$("$d8_bin" --allow-natives-syntax -e 'print(%IsSmi(2**30) ? "false" : "true")')"
  [ "$smi" = "$(need d8 pointer_compression)" ] || { echo "sync-shells: d8 pointer compression differs from the pin" >&2; exit 1; }
}

sync_quickjs() {
  local v c d; v="$(need quickjs version)"; c="$(need quickjs commit)"; d="$out/quickjs-$v"
  [ "$plat" != windows_x64 ] || { echo "quickjs: not built on Windows (skipped)"; return 0; }
  if ! fresh "$d" "$c"; then
    rm -rf "$d"; git_at "$d/src" "$(need quickjs repo)" "$c"
    make -s -C "$d/src" -j "$(getconf _NPROCESSORS_ONLN 2> /dev/null || echo 4)" qjs > "$d/build.log" 2>&1 ||
      { tail -20 "$d/build.log" >&2; exit 1; }
    echo "$c" > "$d/.ep9-stamp"
  fi
  qjs_bin="$d/src/qjs"
}

sync_bun() {
  local v d stamp; v="$(need bun version)"; d="$out/bun-$v"; has_asset bun || return 0
  stamp="$v $(need bun "sha256_$plat")"
  if ! fresh "$d" "$stamp"; then fetch_asset bun "$d"; echo "$stamp" > "$d/.ep9-stamp"; fi
  bun_bin="$(echo "$d"/bun-*/bun"$exe")"
  chmod +x "$bun_bin"
}

sync_brimstone() {
  local v c d; v="$(need brimstone version)"; c="$(need brimstone commit)"; d="$out/brimstone-$v"
  if ! fresh "$d" "$c"; then
    rm -rf "$d"; git_at "$d/src" "$(need brimstone repo)" "$c"
    # Its own rust-toolchain.toml and Cargo.lock apply (cargo runs inside its checkout).
    (cd "$d/src" && CARGO_TARGET_DIR="$d/target" cargo build --release --locked -p brimstone > "$d/build.log" 2>&1) ||
      { tail -20 "$d/build.log" >&2; exit 1; }
    echo "$c" > "$d/.ep9-stamp"
  fi
  bs_bin="$d/target/release/bs$exe"
}

want=("$@"); [ ${#want[@]} -gt 0 ] || want=(node d8 quickjs bun brimstone)
mkdir -p "$out"
for s in "${want[@]}"; do
  case "$s" in
    node | d8 | quickjs | bun | brimstone) "sync_$s" ;;
    *) echo "sync-shells: unknown shell '$s'" >&2; exit 2 ;;
  esac
done

# Record what is installed. quickjs and brimstone have no version flag; their pins name them.
{
  echo "# host: $plat"
  [ -z "$node_bin" ] || echo "node      $("$node_bin" --version) v8=$("$node_bin" -p process.versions.v8)  $node_bin"
  [ -z "$d8_bin" ] || echo "d8        $("$d8_bin" -e 'print(version())')  $d8_bin"
  [ -z "$qjs_bin" ] || echo "quickjs   $(need quickjs version) ($(need quickjs commit))  $qjs_bin"
  [ -z "$bun_bin" ] || echo "bun       $("$bun_bin" --version) ($("$bun_bin" --revision))  $bun_bin"
  [ -z "$bs_bin" ] || echo "brimstone $(need brimstone commit)  $bs_bin"
} | tee "$out/versions.txt"
