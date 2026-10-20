# Reference shells, M0 week 1 (2026-10-20)

- The 5 reference shells of [09-milestones](../plan/09-milestones.md) section 8 week 1 are pinned in `third_party/shells.toml`.
- `bash scripts/sync-shells.sh` installs them into `third_party/shells/` (git-ignored). It checks every download against a pinned SHA-256 and checks pointer compression at run time.
- node 24.22 has no pointer compression. d8 15.8.24 has it. Two independent checks agree for each shell.

## Pins and installed versions

Host: Apple M5 Max, 18 cores, 128 GiB, macOS 27.0.1 (aarch64-apple-darwin).

| Shell | Pin | How it is installed | Version printed after install |
|---|---|---|---|
| node | 24.22.0 | nodejs.org tarball, SHA-256 from SHASUMS256.txt | `v24.22.0`, V8 `13.6.233.17-node.57` |
| d8 | V8 15.8.24 | jsvu 3.0.6 (`jsvu --os=<os> v8@15.8.24`), SHA-256 of the binary | `15.8.24` (`version()`) |
| QuickJS | 2026-06-04 = commit `3d5e064e9dd6` | git fetch of the commit, `make qjs` | `QuickJS version 2026-06-04` (`qjs -h`) |
| bun | 1.4.3 | GitHub release zip, SHA-256 from SHASUMS256.txt | `1.4.3` (`1.4.3+c6da4a4d3`) |
| Brimstone | `64d66980ce03` | git fetch of the commit, `cargo build --release --locked -p brimstone` with its own toolchain file (1.95.0) | binary `bs` (no version flag) |

Notes:
- bellard/quickjs has no git tags. The pin is the "new release" commit that set `VERSION` to 2026-06-04 (`git log -- VERSION`). The research runs used a later commit (`e2dc3eb`) for some cells [R07].
- V8 15.8.24 was the newest canary build on 2026-10-20 (`v8-<os>-rel-latest.json` gave 15.8.24 for mac-arm64, linux64, linux-arm64 and win64).
- jsvu 3.0.6 knows no linux-arm64 V8 build. So Linux arm64 has no d8 in this setup. Windows has no QuickJS build in `sync-shells.sh`.
- bun with `BUN_JSC_useJIT=0` runs JSC without JIT. Check: a 3e7-iteration loop took 0.06 s with the JIT and 0.82 s without.

## Pointer compression check

| Shell | Check 1: build config | Check 2: Smi width at run time | Result |
|---|---|---|---|
| node 24.22.0 | `process.config.variables.v8_enable_pointer_compression` = `0` | `node --allow-natives-syntax -p "%IsSmi(2**30)"` = `true` (32-bit Smis) | no pointer compression |
| d8 15.8.24 | `v8_build_config.json` in the vendor zip: `"pointer_compression": true`, `"pointer_compression_shared_cage": true`, `"sandbox": true` (mac-arm64, linux64 and win64 zips) | `d8 --allow-natives-syntax -e "print(%IsSmi(2**30), %IsSmi(2**30-1))"` = `false true` (31-bit Smis) | pointer compression |

- Why check 2 works: with pointer compression V8 uses 31-bit Smis, so 2^30 is a heap number. Without it, 64-bit V8 uses 32-bit Smis.
- jsvu does not install `v8_build_config.json`, so `sync-shells.sh` runs check 2 after each install. It fails if the result differs from `pointer_compression` in `shells.toml`.
- G7 compares ep9 against "a pointer-compressed d8 --jitless". The run-time flag works on this build: `d8 --jitless -e 'print("jitless ok")'` prints `jitless ok`. (The build config says `has_jitless: false`; that key does not stop the flag.)

## Install runs

| Host | Shells | First run | Second run |
|---|---|---|---|
| macOS arm64 (this machine) | all 5 | 55 s (QuickJS and Brimstone builds) | 0.22 s, no download, no build |
| Linux arm64 (Docker, ubuntu:24.04) | node, quickjs, bun; d8 skipped | 18.5 s | 0.04 s |
| Linux x64 (Docker on Rosetta, ubuntu:24.04) | node, d8, bun | 15.2 s; d8 SHA-256 and the Smi check passed | not run |
| Windows x64 | none | not run (no Windows host here) | |

## Smoke run: v8-v7 combined.js

One run per shell of `bench-v8v7:bench/bench-v8/combined.js` (all 8 kernels), 21:26-21:28 (+08), 1-minute load 17-19 on 18 cores. Every shell exits 0 and prints a score. **One noisy run each: this shows that the shells run the pinned source. It is not a measurement.**

| Shell and mode | Exit | v8-v7 SCORE |
|---|---|---|
| node 24.22 `--jitless` | 0 | 6088 |
| d8 15.8.24 `--jitless` | 0 | 6046 |
| QuickJS 2026-06-04 | 0 | 2905 |
| bun 1.4.3 with `BUN_JSC_useJIT=0` | 0 | 3796 |
| Brimstone 64d6698 | 0 | 5197 (copy with a 1-line `print` shim); 5444 (the pinned file as is, a later run) |

## Reproduce

```bash
bash scripts/sync-shells.sh                 # all shells for this host
bash scripts/sync-shells.sh node d8         # a subset
cat third_party/shells/versions.txt         # versions and paths of the last run
```
