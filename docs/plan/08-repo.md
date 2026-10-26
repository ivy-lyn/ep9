# ep9 plan: repository and Cargo workspace

- One root workspace holds the engine, the runtime, the compilers and the tools. `exclude` keeps `third_party`, `fuzz` and `tools/ep9-typecheck` out of it [R09].
- Dual mode adds 5 crates (`ep9_facts`, `ep9_hints`, `ep9_unit`, `ep9_aot`, `ep9c`) and renames 2 (`ep9_baseline`, `ep9_opt`). The deoptimizer runtime moves into `ep9_vm`.
- No engine crate depends on tsrs; the AOT review upholds this [R16][R17]. tsrs builds twice outside the root workspace: `tsrs_cli` for `ep9 check`, and `tools/ep9-typecheck` for AOT hints.
- The Cargo features `jit` and `aot` give three engine binaries. The jitless build keeps T0, the bytecode compiler, the unit loader and the deoptimizer (H5, H27).
- CI guards keep tsrs pins out of the engine graph and `ep9_hints` at zero dependencies. New xtask checks cover determinism, FP rules (`fp-check`, H36) and zero text relocations.

IDs used here: H2, H3, H5, H7, H13, H14, H16, H24, H26, H27, H30, H32, H34, H35, H36, H37; C10, C11; D1, D2, D4, D8, D12, D16, D17; A1h, A2, A3; K-A4H; M0; RX0, RX1, RX2; T0-T3; SP-tsrs-win; HINT-SET, TYPED-KERNELS, SHORT-RUN, JITLESS-SET; FactSnapshot, HINTED.

Evidence tags: [R02] [R03] [R04] [R08] [R09] [R13] [R14] [R16] [R17] [TSRS]. Paths are in the table at the end.

## 1. Tree

```
ep9/
├─ Cargo.toml            # [workspace] resolver = "3"; members = ["crates/*", "tools/*"]
│                        # exclude = ["third_party", "fuzz", "tools/ep9-typecheck"]   <- required
├─ rust-toolchain.toml   # channel = "1.99.1" (same as tsrs; D8)
├─ .gitignore            # /target/  /third_party/*/  !/third_party/patches/
│                        # /tools/ep9-typecheck/target/  /tools/ep9-typecheck/Cargo.lock      (new)
├─ docs/adr/             # one ADR per H row (H1-H37); CODEOWNERS on contract crates
├─ LICENSE-MIT, LICENSE-APACHE, THIRD-PARTY-LICENSE   # MIT OR Apache-2.0; no NOTICE file (12-governance §1)
├─ .gitattributes        # LF everywhere; *.patch -text; binary list (12-governance §6)
├─ expectations/         # test262.<target>.<mode>.txt (changed), test262-parse.<target>.txt, node-tests.txt, wpt.txt,
│                        # node-test-exclusions (D4), jetstream-m2.txt, jetstream-m7.txt, npm-ranking,
│                        # hint-set.txt, typed-kernels.txt, short-run.txt, jitless-set.txt   (new)
├─ third_party/          # pins and patches unchanged
│  ├─ tsrs.commit        # 745eb10a...  (never a dependency of any root-workspace crate)
│  ├─ test262.commit     # d53ebc8b...
│  ├─ bench-octane.commit    # chromium/octane: Octane-7 kernels, TS workload
│  ├─ bench-v8v7.commit      # boa-dev/data branch benchmarks: bench/bench-v8/combined.js (v8-v7)
│  ├─ bench-jetstream.commit # WebKit/JetStream branch JetStream3.0
│  ├─ shells.toml            # reference shells: version, asset and sha256 per platform (sync-shells.sh)
│  ├─ regress.commit     # base of the RX0 fork
│  ├─ oxc.commit         # only while ep9 carries oxc patches (commit of the 0.155.0 release)
│  └─ patches/<name>/NNNN-*.patch   # <name> = the pin name: tsrs, oxc, regress, bench-jetstream, ...
├─ scripts/
│  ├─ sync-lib.sh        # shared: fetch pin, reset, apply patches in order, write stamp
│  ├─ sync-tsrs.sh       # sync-lib + build tsrs_cli + build tools/ep9-typecheck            (changed)
│  └─ sync-{oxc,regress,test262,bench,shells,node-tests,wpt}.sh   # sync-bench.sh syncs the 3 bench-* pins
├─ crates/               # section 2
├─ tools/
│  ├─ test262/ bench/ difftest/ nodecompat/ xtask/   # root workspace members
│  └─ ep9-typecheck/     # OWN [workspace]; tsrs crates by path; lock copied from tsrs      (new)
└─ fuzz/                 # cargo-fuzz targets (own workspace, nightly)
```

**Names (decision NAMING, 2026-10-20).** One pin per git checkout: `third_party/<name>.commit` pins `third_party/<name>/`, and its patches live in `third_party/patches/<name>/` (sync_dep, §5). Expectation files are `<suite>.<target>[.<mode>].txt`. `<target>` is the Rust target triple (for example `aarch64-apple-darwin`). `<mode>` is the `--force-mode` value (`jit`, `aot-baseline`, `aot-opt`, `hybrid`; only `jit` before A3). `test262-parse.<target>.txt` has no mode, because the front end is the same in every mode (H5).

| Excluded path | Why |
|---|---|
| `third_party` | Path deps under the root become members. The build then fails with "error inheriting edition" [R09]. The `[patch.crates-io]` paths into `third_party/oxc` also need it |
| `fuzz` | Own workspace on nightly |
| `tools/ep9-typecheck` | Own `[workspace]` root with tsrs's exact versions. Inside the root workspace, tsrs's Cargo.lock is ignored and its exact pins leak in [R09] |

The 4 new expectation files pin HINT-SET, TYPED-KERNELS, SHORT-RUN and JITLESS-SET ([10-quality.md](./10-quality.md)).

## 2. Crates

"x" = linked into that binary. "ep9 jitless" is the `ep9` CLI built without `jit`. The iOS library (D16) is the same set without `ep9_cli`.

| Crate | Purpose | unsafe | Feature | ep9 JIT | ep9 jitless | ep9c | ep9-typecheck |
|---|---|---|---|---|---|---|---|
| `ep9_platform` | VA reserve incl. the code range (H7); MAP_JIT/W^X under `jit`; stack bounds; signals/SEH; spill trampolines; unwind registration for JIT code and loaded units | yes | default | x | x | x | - |
| `ep9_value` | NaN-box `Value`, `JsResult`, `Throw`, tags, NaN purification. No deps | yes (Miri) | default | x | x | x | - |
| `ep9_heap` | Cage, pages, GC S0-S3, barrier, CSS, roots, verifier; + unit RW-data visitor hook | yes | default | x | x | x | - |
| `ep9_bytecode` | Opcodes, encoding, verifier, disassembler; + ABI version, determinism, stable function ids, CompiledUnit serialization incl. TS flags and hint section (H2) | no | default | x | x | x | - |
| `ep9_frontend` | The only oxc importer: parse, semantic, depth guard, scope pass, BytecodeGen + TS lowering, hint join from `.ep9h`, lone-surrogate shim, L1 cache | small, contained | default | x | x | x | - |
| `ep9_vm` | Isolate, realms, objects, strings, interpreter, ICs, frames; + deoptimizer runtime, fuse registry, dispatch slots with AOT kinds, pc->code registry (H14), FactSnapshot export | yes (reviewed) | default | x | x | x | - |
| `ep9_builtins` | ECMA-262 spec layer. Until M1a: one crate, one module per test262 area; its engine dependencies are `ep9_vm` and `ep9_math` only (W4-BUILTINS-CRATE; dep-guards check 8). The split into one `ep9_builtins_*` crate per test262 area (C10) is checked again at M1a | forbid | default | x | x | x | - |
| `ep9_numconv` | dtoa ports, radix, StringToNumber, f16 | no (Miri) | default | x | x | x | - |
| `ep9_math` (new) | One Rust Math (H36): CR libm-class functions, `pow_js`, V8 `hypot`, exact `fmod`, `fma_exact`. No dependencies | one fn (fma dispatch); Miri skips corpus tests | default | x | x | x | - |
| `ep9_regexp` | Backend trait. RX0 regress fork; RX1 and RX2 irregexp port | backend only | default; RX2 under `jit` | x | x (RX1) | x | - |
| `ep9_intl` | ECMA-402 on ICU4X | forbid | `intl` | x | x | x | - |
| `ep9_temporal` | temporal_rs glue; shared tz provider for Date | forbid | default | x | x | x | - |
| `ep9_snapshot` | Heap serializer and deserializer, external-reference table; + stable index tables (H13) | yes | default | x | x | x | - |
| `ep9_facts` (new) | FactSnapshot schema (= feedback schema, H16), `.ep9prof` format, HINTED entries for the hint section | no | default | x | x | x | - |
| `ep9_hints` (new) | `.ep9h` reader and writer (H37). **Zero dependencies** | no | default | x | x | x | x |
| `ep9_unit` (new) | Unit container (`.ep9u`, `.o` sections), header and versions, verify; loader (map, register, resolve, install dispatch slots, fallback); writer (`object` crate) | yes (loader) | loader default; writer `aot` | x (loader) | x (loader) | x | - |
| `ep9_masm` | MacroAssembler x64/arm64 with abstract relocation kinds (H34). JIT finalize, code space, dispatch-table writes, jitdump; unit finalize | yes | `jit` or `aot` | x | - | x | - |
| `ep9_baseline` (was `ep9_jit_baseline`) | T1 code generator: JIT emit (`jit`) and baseline-AOT emit (`aot`) | yes | `jit` or `aot` | x | - | x | - |
| `ep9_opt` (was `ep9_jit_opt`) | T2 (later T3) IR, fact model (H30), passes, regalloc, deopt-metadata emission, OSR; JIT emit and optimized-AOT emit. No deoptimizer runtime | yes | `jit` or `aot` | x | - | x | - |
| `ep9_aot` (new) | AOT driver: module graph, `.ep9prof` and `.ep9h` loading, hot-set selection, FactSnapshot assembly; calls `ep9_baseline`/`ep9_opt`; writes units through `ep9_unit` | no | `aot` | - | - | x | - |
| `ep9` | Unstable embedding facade; + unit loading API | minimal | default | x | x | - | - |
| `ep9_runtime` | Event loop, module loader (ESM/CJS/TS), workers, web APIs | no | default | x | x | - | - |
| `ep9_node` | `node:*` modules, Buffer, V8 stack API | no | default | x | x | - | - |
| `ep9_inspector` | CDP server, sampling profiler, heap-snapshot writer | no | default | x | x | - | - |
| `ep9_napi` | Node-API C ABI (D4); stack walks cover AOT frames | yes (FFI) | default | x | x | - | - |
| `ep9_cli` | Binary `ep9`: run, repl, eval, check, snapshot build, test262 host, Fuzzilli REPRL, JSC-shell API; flags `--jitless`, `--unit`, `--profile-out` | no | default | x | x | - | - |
| `ep9c` (new) | Binary `ep9c`: `build --bytecode`, `--baseline`, `--opt`, `--profile`, `--hints`, `--tsconfig`, `--emit obj\|blob`, `--all`; `inspect`; `--sound` only if the D17 rule picks (c) | no | `aot` | - | - | x | - |

- `ep9c` links the engine core. So its compilers use the same intrinsic, builtin and ShapeId tables as the runtime.
- Tools in the root workspace: `tools/test262` (+ `--force-mode`), `tools/bench` (+ AOT benchmark sets), `tools/difftest` (+ mode x tier, hint chaos), `tools/nodecompat`, `tools/xtask` (§7).
- test262 and difftest build twice. `jit` + `aot` runs the jit and hybrid modes. `aot` without `jit` runs the AOT-jitless code kinds with no runtime codegen. That build differs from the shipped jitless `ep9` only by the AOT emitters and `ep9_aot` ([10-quality.md](./10-quality.md) §1).
- `fuzz/` adds targets for the unit loader and the `.ep9h` and `.ep9prof` parsers. CODEOWNERS adds `ep9_unit`, `ep9_aot` and `ep9_facts`.

## 3. Binaries

```
 root workspace (one Cargo.lock)                       outside the root workspace
 +-------------------------------------------------+   +---------------------------------------------+
 | ep9_cli -> ep9 facade -> ep9_runtime -> ep9_vm  |   | third_party/tsrs (tsrs's workspace + lock)  |
 |   always: T0, compiler, ep9_unit loader         |   |   tsrs_cli ----------------> binary `tsrs`  |
 |   `jit`:  ep9_masm, ep9_baseline, ep9_opt       |   | tools/ep9-typecheck (own [workspace])       |
 | ep9c -> ep9_aot -> ep9_baseline, ep9_opt        |   |   tsrs crates (path) + ep9_hints (path)     |
 |   `aot`:  ep9_masm unit finalize, unit writer   |   |   -------------> binary `ep9-typecheck`     |
 | ep9_frontend -> oxc_* =0.155.0, ep9_hints       |   +---------------------------------------------+
 +-------------------------------------------------+
 ep9 check --spawn--> tsrs                 ep9c build --hints --spawn--> ep9-typecheck --> .ep9h
```

| Binary | Workspace | Features | Contains | Leaves out | Serves |
|---|---|---|---|---|---|
| `ep9` (JIT build) | root | default (`jit`) | T0, bytecode compiler, T1, T2, JIT code space, unit loader, deoptimizer, runtime | Unit writer, `ep9_aot` | JIT mode; hybrid mode with `--unit` |
| `ep9` (jitless build) | root | no `jit` | T0, bytecode compiler, unit loader, deoptimizer, runtime | `ep9_masm`, `ep9_baseline`, `ep9_opt`, all runtime codegen | AOT-jitless mode; locked-down hosts |
| iOS library (D16) | root | no `jit` | As the jitless build, without `ep9_cli`; units linked in as `.o` | As above | AOT-jitless mode on iOS |
| `ep9c` | root | `aot` | Engine core, `ep9_aot`, AOT emit in `ep9_baseline`/`ep9_opt`, unit finalize, unit writer | Runtime codegen, event loop, `node:*`, inspector, Node-API | Builds units |
| `ep9-typecheck` | `tools/ep9-typecheck` | n/a | tsrs crates, `ep9_hints` | Every engine crate | Writes `.ep9h` |
| `tsrs` | `third_party/tsrs` | n/a | `tsrs_cli` | Every ep9 crate | `ep9 check` |

- **Why jitless keeps the compiler and T0.** eval, `new Function` and `toString` must work in every mode (H5). Hermes' lean VM without its compiler aborts; the compiler is 2.4 MB of its 6.0 MB [R14].
- **Why jitless keeps the deoptimizer.** Optimized-AOT code deopts to T0 or baseline-AOT, and debugger attach deopts AOT frames (H24, H32).

## 4. Performance note (unchanged)

The hot code stays in one crate, `ep9_vm`: interpreter, ICs, shapes, strings and builtin fast paths. Release builds use fat LTO and `codegen-units = 1`, as the parser benchmarks did [R03] (C10).

## 5. How `third_party/tsrs` is consumed

1. **No root-workspace crate depends on tsrs.**
   - Its exact pins (`unicode-id-start =1.1.2`, `icu_normalizer =2.3.0`) would enter the engine graph. oxc would drop to Unicode 15.1 and fail 16 test262 identifier tests [R02][R09].
   - In-process use also brings a fixed 32 GiB reservation at 0x4001_0000_0000, arenas that live until exit, and about 220 globals [R09][R13].
2. **`exclude = ["third_party"]` stays** (§1).
3. **Two builds outside the root workspace** (C11):

| Build | Workspace | Lock | Toolchain | Output | Used by |
|---|---|---|---|---|---|
| `tsrs_cli` | tsrs's own (`third_party/tsrs`) | tsrs's Cargo.lock | tsrs's `rust-toolchain.toml` (1.99.1) | `tsrs` | `ep9 check` |
| `ep9-typecheck` | `tools/ep9-typecheck`, empty `[workspace]` | tsrs's lock, copied; unused entries pruned; 2 ep9 entries added | Same channel as tsrs (1.99.1, H26) | `ep9-typecheck` | `ep9c build --hints` |

4. **ep9-typecheck depends on tsrs crates and `ep9_hints` only.** Both are path deps. `ep9_hints` has zero dependencies, so the tool adds only 2 packages to tsrs's graph.
5. **Discovery.** `ep9 check` finds `tsrs` through `EP9_TSRS`, else next to the `ep9` binary. `ep9c` finds `ep9-typecheck` through `EP9_TYPECHECK`, else next to `ep9c`.
6. **CI sync-check.** It runs on the unix targets when any `*.commit` or patch file changes. It re-syncs, re-applies patches and builds both tools. It runs `ep9 check` smoke tests and a hint-export golden test (`.ep9h` compared byte for byte).
7. **SP-tsrs-win (M0 week 1, 0.25 PM est.).** Build `tsrs_cli` on x86_64-pc-windows-msvc and run the smoke tests.
   - Upstream builds a Windows x64 release binary and smoke-tests it (`--version`, exit 2 on a type error) [TSRS .github/workflows/release.yml:292-408]. Its one per-PR Windows CI job checks `tsrs_checker` and runs the `tsrs_api_transport` tests [TSRS .github/workflows/ci.yml:43-55]. The Windows release job (only when a release is cut) also runs the full conformance suite as PGO training; it fails on a crash, a timeout or fewer than 10,000 cases [TSRS .github/workflows/release.yml:295, :336-341; .github/scripts/pgo-train-windows.ps1:14-26]. On Windows tsrs uses plain pointers, because compressed_ptrs is unix-only [TSRS crates/tsrs_core/build.rs].
   - Budget: ≤ 2 local patches. It runs again for `ep9-typecheck` in A1h. On failure, `ep9 check` on Windows prints "not supported" and `ep9c --hints` stays unix-only (D2).
8. **Oracle build facts** [R13]. The prototype linked 21 tsrs crates (363k lines) through the public API, with no tsrs change. A clean build took 38 s; the stripped binary is 13.6 MB.
   - Drift: a 2-line fix against an 8-day-old tsrs, 6 compile errors against a 15-day-old one. Budget: ≤ 2 local tsrs patches (est., D12); upkeep above 1 PM per year trips K-A4H.
   - Limit: without tsrs regions, the fixed 32 GiB range aborted at about 4,800 composite projects (issue #597). Large monorepos may need one run per project.
9. The local `tsrs_cli` build has no PGO or BOLT; tsrs release binaries have both [TSRS Cargo.toml:86-89]. Type-check speed is not an engine gate.

`scripts/sync-lib.sh` (shared by every `sync-<name>.sh`; v1 plus `core.autocrlf=false` in each checkout, [12-governance §6](./12-governance.md#6-line-endings)):

```bash
# usage: sync_dep <name> <git-url>; caller sets: set -euo pipefail; root=...
patch_hash() { { for p in "$1"/*.patch; do [ -e "$p" ] && cat "$p"; done; true; } | git hash-object --stdin; }
sync_dep() {
  local name=$1 url=$2 dir="$root/third_party/$1" pdir="$root/third_party/patches/$1" rev want
  rev="$(tr -d '[:space:]' < "$root/third_party/$name.commit")"
  want="$rev $(patch_hash "$pdir")"
  [ -d "$dir/.git" ] || { git init -q "$dir"; git -C "$dir" remote add origin "$url"; }
  git -C "$dir" config core.autocrlf false           # LF on every OS: .gitattributes stops at this work tree
  [ "$(cat "$dir/.ep9-stamp" 2>/dev/null || true)" = "$want" ] && return 0
  git -C "$dir" fetch -q --depth 1 origin "$rev"
  git -C "$dir" checkout -q -f --detach FETCH_HEAD   # drops patches from earlier runs
  git -C "$dir" clean -q -fd                         # keeps ignored target/
  for p in "$pdir"/*.patch; do [ -e "$p" ] || continue; git -C "$dir" apply --whitespace=nowarn "$p"; done
  echo "$want" > "$dir/.ep9-stamp"                   # re-runs when the pin OR the patch set changes
}
```

`scripts/sync-tsrs.sh` (changed: it also builds `ep9-typecheck`):

```bash
#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/sync-lib.sh"
sync_dep tsrs https://github.com/oxc-project/tsrs.git
tsrs="$root/third_party/tsrs"; tc="$root/tools/ep9-typecheck"
chan="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$tsrs/rust-toolchain.toml")"   # 1.99.1 today
# 1) `ep9 check`: tsrs_cli in tsrs's own workspace (its lock, profile, toolchain; no PGO/BOLT)
(cd "$tsrs" && cargo build --release --locked -p tsrs_cli)
# 2) AOT type oracle: own empty [workspace]; start from tsrs's exact lock, add only ep9 entries
cp "$tsrs/Cargo.lock" "$tc/Cargo.lock"
(cd "$tc" && cargo "+$chan" update --workspace && cargo "+$chan" build --release --locked)
echo "tsrs:          $tsrs/target/release/tsrs"
echo "ep9-typecheck: $tc/target/release/ep9-typecheck"
```

- `cargo update --workspace` adds the `ep9-typecheck` and `ep9_hints` entries and drops unused tsrs entries. It keeps every other version. The lock guard (§7) checks this.

## 6. How oxc is consumed (unchanged)

- From crates.io with `=0.155.0` pins (D1 = E).
- One bump PR at a time, about every 8 weeks. Each bump carries a test262 diff and an instruction-count diff [R04].
- Local patches only through the oxc track ([01-front-end.md](./01-front-end.md); ≤ 3 patches, each linked to an upstream PR). The depth-counter patch is the most likely one.
- `oxc_transformer` runs only for TS legacy decorators (D4). Those files get no hints. All other TS lowering is in BytecodeGen (H3).

## 7. Dependency guards and xtask checks

| Guard (every PR) | Rule | Evidence |
|---|---|---|
| Unicode pin | `cargo tree -i unicode-id-start` in the root workspace shows only ≥ 1.5.0 (Unicode 17) | [R02] |
| No tsrs in the engine | `cargo tree -i tsrs_core` is empty for the whole root workspace | [R09][R13] |
| `ep9_hints` | 0 normal and build dependencies | [R13] |
| ep9-typecheck lock | Every package in its Cargo.lock has the same version as in tsrs's lock. Only `ep9-typecheck` and `ep9_hints` are new | [R09] |
| Jitless build | The shipped jitless `ep9` has no `jit` feature and no `ep9_masm`, `ep9_baseline` or `ep9_opt`. The `aot`-only test build is exempt | H27 |
| `object` crate | Exact `=` pin; version chosen at A2 | H35 |
| cargo deny | Ban LGPL crates (malachite, rug). Track MPL file-level copyleft and tsrs's Apache-2.0 NOTICE; the NOTICE ships with `ep9-typecheck`. Licence allow list and notice rules: [12-governance §1](./12-governance.md#1-licence) | [R04][R08][R13] |

| xtask check | What it verifies | From |
|---|---|---|
| v1 checks (unchanged) | Tail-call disassembly (every release handler ends in an indirect branch, Win64 included); ≤ 4 handler arguments; size asserts (`Value`, `JsResult`, word0 = 8 B); ADR lint for H-row crates; pin-leak guard (the guards above) | M0 |
| Determinism (new) | Compile the test262 + bench corpus twice per target. CompiledUnit hashes must match (H2) | M0 week 3; gate at M0 exit |
| Unit golden files (new) | The H35 header struct and container spec v0 match committed golden files | M0 exit |
| Feature matrix (new) | `default` (`jit`), jitless (no `jit`) and `aot` build and test on 4 targets (H27) | M0 week 4 |
| fp-check (H36, new) | No banned libm symbol, no FMA instruction outside `ep9_math`, no second math crate; 5 targets; exceptions in `expectations/fp-check-allow.toml`. Root `clippy.toml` bans the std libm methods, `powi`, `rem_euclid`, `div_euclid`, `mul_add` and `algebraic_*` | Stub M0 week 2 |
| Zero text relocations (new) | Each unit text section has 0 relocations; the writer rejects HeapConst (H34, H35) | A2 |
| `ep9_hints` zero-deps (new) | The `ep9_hints` manifest lists no dependencies (H37) | A1h |

## Changes vs plan v1

| Area | v1 | v2 |
|---|---|---|
| `exclude` | `third_party`, `fuzz` | + `tools/ep9-typecheck` |
| New crates | n/a | `ep9_facts`, `ep9_hints`, `ep9_unit`, `ep9_aot`, `ep9c` |
| Renamed crates | `ep9_jit_baseline`, `ep9_jit_opt` | `ep9_baseline`, `ep9_opt` (JIT and AOT emit) |
| Deoptimizer runtime | In the JIT crates | In `ep9_vm`, linked in every build |
| tsrs builds | `tsrs_cli` only | + `tools/ep9-typecheck` (own workspace, copied lock); `sync-tsrs.sh` builds both |
| Binaries | `ep9` (JIT, jitless) | + `ep9c`, `ep9-typecheck`, iOS library (D16); composition table |
| Expectations | per target | per target and mode; 4 new benchmark set files |
| Guards and xtask | 3 guards, 5 xtask checks | + 4 guards, + 6 xtask checks |
| Unchanged | | `sync-lib.sh`, pins, patch flow, oxc pin policy, performance note |
| Decisions 2026-10-20 | | Pin and expectation names (NAMING); `ep9_math` (H36); `fp-check` replaces the FMA lint; licence files and `.gitattributes`; `--sound` by the D17 rule. See [decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction) |

## Evidence tags

| Tag | Path |
|---|---|
| [R02] | [../research/02-tsrs-conformance.md](../research/02-tsrs-conformance.md) |
| [R03] | [../research/03-parser-perf-and-features.md](../research/03-parser-perf-and-features.md) |
| [R04] | [../research/04-parser-alternatives.md](../research/04-parser-alternatives.md) |
| [R08] | [../research/08-spec-subsystems.md](../research/08-spec-subsystems.md) |
| [R09] | [../research/09-critic.md](../research/09-critic.md) |
| [R13] | [../research/13-tsrs-type-oracle.md](../research/13-tsrs-type-oracle.md) |
| [R14] | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R16] | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R17] | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) (wins over [R16]) |
| [TSRS] | `third_party/tsrs` at `tsrs.commit` (path:line allowed) |
