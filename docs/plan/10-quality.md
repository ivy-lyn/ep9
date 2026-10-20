# ep9 plan: testing, benchmarks, fuzzing, CI and risks

- test262 runs in every shipped mode and code kind, through the real unit loader. The pass set must be equal everywhere (G12).
- New suites check bytecode determinism, unit version fallback, FP results against V8, hint soundness and profile staleness.
- Six pinned benchmark sets drive the AOT gates. Every gate names its comparator.
- PR regression gates use instruction counts. Spike, cross-engine, startup and warm-up gates use wall time on a gate host, under the speed-gate rule in §3.
- Fuzzing adds a mode x tier differential, hint chaos, and the parsers for units, .ep9h and .ep9prof.
- Risks: RK1-RK28 come from plan v1. RK29-RK44 are new AOT risks.

IDs used here: G1, G6, G7, G12, G13, G15; N10; H2, H3, H5, H11, H16, H29-H37; D1-D21; M0-M10, M1a, M5c; A1h, A1, A2, A3, A4, A5, A5v; T0-T3; baseline-AOT, optimized-AOT; JIT mode, AOT-jitless mode, hybrid mode; PROVEN, RUNTIME, PROFILED, HINTED; SLOWPATH, PIN, THROW; FV1-FV3; K-A3, K-A4J, K-A4P, K-A4H, K-A5pre, K-A5, K-A5v-cov, K-A5v-bound, K-A5v; VERIFIED; V1-V3 (barrier verifiers); HINT-SET, TYPED-KERNELS, SHORT-RUN, JITLESS-SET, STARTUP-SET, SIZE-REPORT; S1, S2, S2b, S3; RX0, RX1; P1-P5; SP-*, Spike B; RK1-RK44.

Evidence tags: [R01] [R02] [R03] [R04] [R05] [R06] [R07] [R08] [R09] [R10] [R11] [R12] [R13] [R14] [R15] [R16] [R17] [R18] [P1] [T262] [TSRS]. Paths are in the table at the end.

---

## 1. test262 runner

Plan v1 rules stay:
- One process with rayon workers. A fresh realm per test. Harness files are pre-compiled per worker, later from a snapshot [R08].
- Strict and sloppy runs (103,175 runs). Module, raw, async and negative-phase support.
- Results count per file by the G1 rule, and also per run. node and QuickJS run once through it to calibrate against test262.fyi.
- The full `$262` API: createRealm, detachArrayBuffer, evalScript, gc, IsHTMLDDA, AbstractModuleSource, and agents on real threads (109 agent tests) [R08].
- A watchdog interrupt per test. Brimstone's harness had none, and one test hung it for 11 minutes [R07].
- Subprocess shards, so a crash loses one shard. A failed shard re-runs test by test.
- Target: full run ≤ 10 min at M0, under 60 s later (est.). QuickJS takes 145 s with one process per test [R07].

New: `--force-mode`. `--force-tier=baseline|opt` (v1) stays for the JIT tiers.

```
tools/test262 --force-mode=<mode>          (each test file, in-process)

test.js -> ep9_frontend -> CompiledUnit
              |
              +- jit           finalize -> T0 -> T1 -> T2 (normal tier-up)
              +- aot-baseline  ep9_aot -> baseline-AOT blob  --+
              +- aot-opt       ep9_aot -> optimized-AOT blob --+--> real ep9_unit loader --> run
              +- hybrid        ep9_aot -> unit = start tier  --+    (map, verify, register,
                               hot code: instrumented T1 -> T2       resolve cells, install slots)
              |
result -> diff against expectations/test262.<target>.<mode>.txt
```

- Every blob goes through the real loader. There is no test-only load path.
- The driver uses `--all`, so every function in the file gets the forced code kind. eval and `new Function` code still compiles at runtime in every mode (H5).
- aot-opt compiles from PROVEN facts plus a PROFILED FactSnapshot from a T0 run of the same file. M7 exit item 6 checks the same path for T2.
- Two runner builds ([08-repo](./08-repo.md) §2):
  - `jit` + `aot`: modes jit and hybrid.
  - `aot` without `jit`: aot-baseline and aot-opt (AOT-jitless mode). It has no runtime codegen. It differs from the shipped jitless `ep9` only by the AOT emitters and `ep9_aot`.
- At the A3 gate and at M8 exit, `ep9c` also builds units for the corpus, and the shipped jitless `ep9` loads them. Its pass set must equal the in-process result.
- Expectations are committed per target and per mode. CI reports only differences. `<target>` = Rust target triple; `<mode>` = `--force-mode` value; the parse-only suite uses `test262-parse.<target>.txt` ([08-repo](./08-repo.md) §1).
- The pass set must be equal across modes and code kinds (G12). A difference is a bug. It never becomes a new expectation.

---

## 2. Suites

| Suite | Purpose | Pin | Cadence | Gate |
|---|---|---|---|---|
| test262 (strict + sloppy) | Conformance | `test262.commit` (d53ebc8b) | Every PR | No new failures against expectations |
| test262 under GC-stress | GC correctness | same | Nightly; PRs touching heap/vm | 0 new failures |
| test262 tier matrix (`--force-tier`, jitless build) | JIT correctness | same | Nightly from M5 | Same pass set in every tier |
| **test262 mode x code-kind matrix** (`--force-mode`) | AOT and hybrid correctness | same | Nightly from A3 (baseline-AOT) and A4 (optimized-AOT, hybrid) | Same pass set in every shipped mode x code kind (G12) |
| Concurrent-marking verifier (`--force-tier`, `--force-mode`) | S2b correctness incl. JIT and AOT stores | same + v8-v7 | Nightly from M5; AOT modes by M8 exit | 0 verifier failures ([03-gc](./03-gc.md)) |
| **Barrier checks** (H11) | Barrier model test (exhaustive interleavings, 6 scenarios incl. `los`). Remembered-set verifier V1 before every minor GC. End-of-marking verifiers V2 (closure) and V3 (independent re-mark). 5 negative controls: drop 1 in N barriers, old-only filter, no root rescan, elide after escape, no card rescan at termination | in-tree + test262 + v8-v7 | Model test and controls: every PR touching the barrier or a verifier, from M0. V1 nightly under GC-stress from S1 (M1); V2, V3 from S2 (M5) | Model test: the decided form passes, the rejected forms fail. Each control makes V1, V2 or V3 fail. 0 verifier failures ([03-gc](./03-gc.md) §7) |
| **Unit version checks** | Mutate every header field and every section of a unit | in-tree unit corpus | Every PR touching `ep9_unit`, from A2 | 100%: each mutation ends in the H35 S3 outcome of its field (bytecode, source section or clean load error). Never a crash, never a wrong result |
| **Bytecode determinism** | Two builds per target; compare bytecode hashes (H2) | test262 + bench corpus | Every PR | Identical hashes |
| **FP differential** | nbody and raytracer checksums equal V8 (nbody -0.169193895234998, raytracer 576888) [R15]. Math results equal T0 in every tier and code kind. FP fuzz (§4). `ep9_math` golden + hash tests every PR on 4 targets and x86-64 v1/v3; mpmath CR oracle and d8/node ULP check nightly | in-tree + `bench-octane.commit`, `bench-v8v7.commit` | Checksums every PR; fuzz nightly; from M1 | 0 mismatches (H36) |
| **Hint soundness corpus** | 24 hole classes [R12] and 13 tsrs hint holes [R13] as tests | in-tree | Every PR touching hints, from A1h | Output with hints on equals node |
| **Profile staleness** | Edited source, wrong source hash, wrong schema version, profile from another app | in-tree | Every PR touching `ep9_facts`/`ep9_aot`, from A3 | Output equals node. Stale functions ignore the profile |
| Octane-7 + v8-v7 kernels | Interpreter/JIT speed | `bench-octane.commit`, `bench-v8v7.commit` | Every PR (instruction counts) | Against the parent: ≤ 1% geomean and ≤ 3% per kernel. Against a ratcheted baseline: ≤ 2% total drift (est.). Or an ADR-noted waiver |
| Fast-path differential tests | No spec deviation in fast paths (N7) | in-tree | Every PR from M2 | 0 mismatches |
| Octane TypeScript workload | Short compiler-like runs | `bench-octane.commit` | Nightly | Milestone gates |
| JetStream 3.0 JS workloads (cli.js + JSC-shell host API) | Broad modern speed | `bench-jetstream.commit` (JetStream3.0 branch), unpatched; lists in `expectations/` | Nightly from M2 | M2 list: ratio recorded. M7 list: G4 |
| **AOT benchmark sets** (§3) | FV and K gates | `expectations/*.txt` | Nightly from the stage that needs them | §3 |
| Startup (STARTUP-SET) | Startup and front end | in-tree corpus | Every PR from M4 (instruction counts); nightly wall time, per mode from A1 | M4 gate; A1 gates; P3/P5 |
| Memory (peak RSS, best of N) | NaN-box cost; per-mode RSS | v8-v7, JetStream | Nightly | Both G7 caps from M2. Per-mode report from A1 (G15) |
| **Size (SIZE-REPORT)** | Binary size per build config, unit size per mode, native/bytecode ratio | in-tree | Nightly from A1 | G15 report. A3 ratio gate |
| Microbenchmarks (dispatch vs cgoto.c, IC hit, allocation, barrier) | Unit-level speed | in-tree | Every PR | Per-benchmark budget |
| node/test/parallel subsets, WPT subsets | Runtime conformance | node tag, WPT commit | Nightly from M4 | M4/M6 gates ([07-runtime](./07-runtime.md)) |
| Real apps (tsc, prettier, D5 app gate, napi-rs suites, npm top-1,000) | Runtime acceptance | pinned versions | Nightly; JIT mode from M4, AOT-jitless from A3, hybrid from A4 | Output equals node in each shipped mode |

- **Why instruction counts.** Wall time was too noisy: load average was 11-37 during the research [R03][R09]. Perf jobs never run on agent hosts. Instruction counts cannot compare two engines (other code, other IPC), so cross-engine ratios use wall time under the §3 speed-gate rule.
- **Why wall time for startup and warm-up.** These costs include page faults, file I/O, unit mapping and background compile threads. Instruction counts do not show them well. So startup and warm-up gates use wall time on a gate host, best of N (class W of the §3 speed-gate rule).
- **Platforms.** Speed gates are hard on macOS arm64 and Linux x64. Windows x64 is compared with Linux x64 on the same hardware class (≤ 10% slower, est.). Linux arm64 is tracked.
- **Reference shells** come through jsvu: d8 with `--jitless` and `--max-opt=0/1/2`, JSC with `useJIT=0` [R06][R07]. node 24.22 is built without pointer compression [P1].
- **JetStream harness.** Every shell runs the same unpatched checkout through the JSC-shell host API (node: `tools/bench/jetstream/node-host.mjs`). The harness Proxy (`throwOnAccess`, JetStreamDriver.js class Scripts) is touched only at setup. It needs the M1 Proxy core (no-trap [[Set]] and [[HasProperty]] forwarding plus the get trap). Only if Proxy misses M2 exit: one patch under `third_party/patches/bench-jetstream/` replaces it with `Object.create(null)`. Sync applies it to the one checkout, so every shell, node included, runs it.

---

## 3. Benchmark sets

All sets are pinned in `expectations/` (`hint-set.txt`, `typed-kernels.txt`, `short-run.txt`, `jitless-set.txt`, plus startup and size lists).

| Set | Content | Comparators | Feeds |
|---|---|---|---|
| HINT-SET | 4 groups: TypeScript compiler sources as a tsc-like run; typed AWFY port; vscode base + editor common; hono | the same ep9 mode with hints off; for regions, AOT-jitless A4 optimized-AOT | SP-HintPrecision, K-A4H, SP-RegionCoverage (K-A5v-cov), K-A5v |
| TYPED-KERNELS | nbody, richards, raytracer, deltablue (typed ports) | V8 full at peak; ep9 T2; ep9 A4 hinted code | K-A5pre, SP-RegionBound (K-A5v-bound), FV3, K-A5, K-A5v |
| SHORT-RUN | tsc 6 on a pinned ~70k-line TS project, 1-2 s [R15]; the Octane TypeScript workload | node with a warm compile cache (cold node reported too); ep9 JIT-only with code cache | FV2, K-A4P, K-A4H; latency report |
| JITLESS-SET | Octane-7, v8-v7 kernels, TS workload | node --jitless; ep9 T0; ep9 baseline-AOT | FV1, K-A3, K-A4J |
| STARTUP-SET | `ep9 -e 0`; typescript.js (9.1 MB) + 1 call; tsc on a small project | node; ep9 cold source path | A1 (G13), M4 startup gate, G15 |
| SIZE-REPORT | binary size per build config; unit size per mode; native/bytecode ratio | per-app payload vs total shipped size | A3 ratio gate, G15 |

When each set decides something:

```
stage ->        A1 (M4)       A1h (by M4)      A3                M7              A4 (gates M8)    A5 (M10)
STARTUP-SET    A1 / G13 ----------------------------------------------------------> G15 report
HINT-SET                      SP-HintPrecision, K-A5v-cov ----------------------> K-A4H --------> K-A5v
SHORT-RUN                                                                         FV2 / K-A4P, K-A4H
JITLESS-SET                                    FV1 / K-A3 ------------------------> K-A4J
TYPED-KERNELS                                                    K-A5pre, K-A5v-bound ----------> FV3 / K-A5, K-A5v
SIZE-REPORT    G15 report ---------------------> A3 ratio gate --------------------> G15 report
```

Gates by set (all thresholds est.; full kill rules in [05-aot](./05-aot.md)):

| Gate | Set | Pass condition | Stage |
|---|---|---|---|
| A1 / G13 | STARTUP-SET | typescript.js + 1 call loads ≥ 2x faster from a unit than the ep9 cold source path; snapshot start ≥ 2x; warm tsc with cache ≥ 1.5x | A1 (M4) |
| FV1 / K-A3 | JITLESS-SET | AOT-jitless ≥ 1.6x node --jitless and ≥ 1.6x ep9 T0. K-A3 trips if baseline-AOT < 1.3x T0 | A3 |
| K-A4J | JITLESS-SET | optimized-AOT (jitless, PGO) ≥ 1.5x baseline-AOT | A4 |
| FV2 / K-A4P | SHORT-RUN | wall time ≤ 0.9x node with a warm compile cache; time-to-peak ≥ 15% better than ep9 JIT-only with code cache; steady state within 2%; test262 equal | A4 |
| AOT vs own JIT (report, no gate) | JITLESS-SET, HINT-SET, TYPED-KERNELS | optimized-AOT (jitless build, PROFILED + HINTED) vs T2 (JIT mode): steady-state ratio and time-to-peak. D15 input at the M8 AOT ship review | A4 |
| K-A4H | HINT-SET, SHORT-RUN | ≥ 3% geomean wall-time gain on HINT-SET in at least one mode, or ≥ 10% better time-to-peak on SHORT-RUN; and no HINT-SET workload > 1% slower | A4 |
| K-A5v-cov | HINT-SET (3 real-app groups) | hot-time share in finder-v1 regions ≥ 15% on ≥ 2 of 3 groups; the share is the lower of a V8-default and a `--jitless` profile | A1h |
| K-A5pre | TYPED-KERNELS | T2 with guards removed (test-only trusted mode) ≥ 1.15x normal T2 | M7 |
| K-A5v-bound | TYPED-KERNELS | region code ≥ 1.1x A4 code inside regions (SP-RegionBound, jitless) | M7 |
| FV3 / K-A5 | TYPED-KERNELS | ≥ 1.1x V8 full at peak on ≥ 2 of 4 kernels, and ≥ 1.3x ep9 A4 hinted code. K-A5 applies to D17 (c) only | A5 (M10) |
| K-A5v | HINT-SET real-app groups, TYPED-KERNELS | real-app part: ≥ 3% geomean end-to-end gain vs A4 optimized-AOT (AOT-jitless); kernel part: FV3. Stop A5v only if both parts fail; a failed part drops only its claim | A5 (M10), D17 (d) only |
| A3 size | SIZE-REPORT | unit text ≤ 8x the bytecode of compiled functions | A3 |
| G15 | STARTUP-SET, SIZE-REPORT | per-app payload, total shipped size, startup and RSS reported per mode; no size target before the A3 review | from A1 |

Measurement rules:
- **Time-to-peak** = wall time from process start to the first iteration within 5% (est.) of steady state. It comes from in-process per-iteration curves [R15].
- **CPU next to wall time.** SHORT-RUN reports user + system CPU too. V8 spent 1.58 s of background compile CPU inside a 1.62 s tsc run [R15].
- **Latency.** SHORT-RUN reports, per mode, the per-iteration max and p99 divided by the median, and the deopt count. D15 (a) claims predictable latency only where this metric shows it. It goes into the G15 report.
- **Same source.** V8 runs the typed ports with types stripped, in the same structure. On the original raytracer source V8 is 2.2x slower (213 vs 96 ms) [R15].
- **Named comparator.** "V8 full" = node 24.22 default. "V8 full, cached" = the same with a warm `NODE_COMPILE_CACHE` (FV2). "Maglev cap" = node --max-opt=2. "jitless" = node --jitless. Every gate result names one of these. node's compile cache alone loads typescript.js 2.0x faster [R10][R17].
- **Noise.** Research cells carry ±10-20% on shared machines [R11][R15]. Week-1 cells ran at load 10-70, with single-workload spreads up to 97%. The speed-gate rule below decides how a gate is measured. One cell never decides a gate alone.

**Speed-gate rule (decision PERF-NOISE, 2026-10-20).**

| Class | Gates | Metric | Host | Rounds |
|---|---|---|---|---|
| R regression | PR perf gate against the parent; ratchet | user-mode instructions: callgrind by default on hosted CI runners (they usually expose no PMU); `perf stat -e instructions:u` where the PMU is visible | any Linux CI runner | 1 (deterministic) |
| S spike | ratios between variants or builds of one program: SP-Dispatch, Spike B round trip, SP-Facts speed | wall time of the timed region | gate host; x64 targets may use a hosted CI runner. The result names its CPU model and holds for that model only | ≥ 15 paired rounds + 1 warm-up round dropped; ≥ 31 on a hosted runner |
| M cross-engine | M0 exit 4, M1-M8 speed exits, JetStream lists, JITLESS-SET, FV and K gates, G4 | wall time (the benchmark's own score or timer) | gate host only | ≥ 10 paired rounds per kernel; JetStream ≥ 7 repetitions |
| W startup, warm-up | M4 startup, A1/G13, FV2 time-to-peak | process wall time | gate host only | N ≥ 20; best and median reported; the gate uses best of N |
| Mem | G7 caps, RSS reports | peak RSS | any host that passes the load guard; comparator on the same host | ≥ 5; median |

- **Gate host.** A bare-metal machine that runs nothing else during the run: no agents, no builds, one CI concurrency group `perf`. AC power, no low-power mode. Linux: `performance` governor, turbo off, the benchmark pinned to isolated cores with the SMT sibling off (LLVM Benchmarking guide). macOS: no pinning; the load guard must pass. Each host has a file `tools/bench/hosts/<host>.toml`: CPU, cores, OS and the idle baseline (median 1-minute load over 10 idle minutes).
- **Load guard.** Check before the run (after 60 s with no benchmark process) and 120 s after it: 1-minute load ≤ idle baseline + 0.5. If it fails, wait up to 30 min, then stop with "host busy". The result records both values.
- **Paired rounds.** Each round runs every config once, back to back, in rotated order. The statistic is the per-round ratio (JetStream: the per-repetition list ratio).
- **Verdict.** Gate value = median of the paired ratios. The 95% interval of the median comes from order statistics: n = 15 → ranks 4 and 12 (96.5%); n = 10 → ranks 2 and 9 (97.9%); n = 7 → min and max (98.4%); n = 31 → ranks 10 and 22 (97.1%). Pass: the whole interval is on the pass side. Fail: the whole interval is on the fail side. Otherwise "marginal": run once more with 2x rounds. The median then decides, and the result keeps the label "marginal".
- **Void.** The host was not quiet when the IQR of the paired ratios is > 5% of the median (classes S and M, per kernel), or the JetStream per-repetition list-ratio range is > 10%. Run again.
- **Provisional.** A result from any other host names its host and load, and never passes or fails a gate. The week-1 SP-Dispatch a64 timings (load 16-21) and the JetStream max-opt ratio (load 17-37) are provisional. Measure them again under this rule before M0 exit item 5 and M7 exit 3 use them.
- **Until gate hosts exist.** The shared macOS arm64 host may act as gate host in a quiet window: the orchestrator runs the perf step alone, and the load guard passes. Class M gates on Linux x64 need a bare-metal Linux x64 gate host by M1 exit. Windows x64 ratios against Linux x64 use the same machine (dual boot) or identical hardware.
- **Owner action.** Buying or renting a bare-metal gate host is an owner action. The orchestrator reports the need as a statement. If no bare-metal Linux x64 gate host exists at M1 exit, class M Linux gates read "marginal (no gate host)", never "pass".

---

## 4. Fuzzing

| Target | Tool | From | Exit metric |
|---|---|---|---|
| Front end (source → verified bytecode) | cargo-fuzz; the dictionary has every deep form of G6 | M0 | No panics, no aborts; the bytecode verifier passes |
| Number conversions | Diff against V8 output tables (400k cases [R08]) | M0 | 0 mismatches |
| RegExp compile + exec; oxc vs matcher validity | cargo-fuzz + diff against node | M1 | 0 mismatches |
| Whole engine | Fuzzilli REPRL + SanitizerCoverage in `ep9_cli` [R09] | M1 | CPU-hour budgets below, 0 unique crashes |
| Output differential | ep9 vs node and QuickJS on generated programs | M1 | Triaged mismatches = 0 |
| **FP differential** | Generated FP programs; every tier and code kind vs node | M1 | 0 mismatches |
| Tier differential + deopt | Fuzzilli + `--force-tier`, concurrent-marking verifier on | M5 (Spike C uses it in M1) | 0 mismatches |
| **Mode x tier differential** | Fuzzilli + `--force-mode` across T0, T1, T2, baseline-AOT, optimized-AOT, hybrid | A3 | 0 mismatches |
| **Hint chaos** | Random wrong hints and stale profiles injected into FactSnapshots | A4 (join part from A1h) | Output never changes |
| **Unit loader** (.ep9u blobs, .o sections) | cargo-fuzz with header and section mutations | A2 | No panics; each input ends in its H35 S3 outcome (bytecode, source section or clean load error) |
| **.ep9h and .ep9prof parsers** | cargo-fuzz | A1h (.ep9h), A3 (.ep9prof) | No panics; bad input drops the hints or the profile |
| JSON, ValueSerializer, snapshot loader, code cache | cargo-fuzz | M3-M6 | No panics |

Crash-free budgets grow from 24 CPU-h (front end, M1) to 100 (M1), 500 (M2), 2,000 (M3), 1,000 tier-diff (M5), 10,000 (M7) and 50,000 cumulative (M8). New: 1,000 CPU-h mode-diff at A3 and 5,000 at A4 (est.).

---

## 5. Other CI checks

| Check | What | When |
|---|---|---|
| Build + clippy `-D warnings` | 4 targets. Feature matrix: default (`jit`), jitless (no `jit`), `aot` | Every PR on its tier's targets; all 4 targets nightly, on `ci:full` and on dispatch ([12-governance §3](./12-governance.md#3-ci-triggers-and-cost)) |
| Tail-call disassembly | Every release handler ends in an indirect branch and takes ≤ 4 arguments; deep-loop stack test; Win64 included | PRs touching `ep9_vm` dispatch |
| Golden | Bytecode disassembly snapshots. Unit golden files (H35 header struct, container layout) | Every PR |
| Determinism | Two builds give identical bytecode hashes (H2) | Every PR |
| Zero text relocations | Every unit emitted in CI has no text relocation; the writer rejects HeapConst (H34) | Every PR touching `ep9_masm`, `ep9_unit`, `ep9_baseline`, `ep9_opt`, from A2 |
| FP lint (H36) | Root `clippy.toml` bans libm, `powi`, `rem_euclid`, `div_euclid`, `mul_add` and `algebraic_*` methods. `cargo xtask fp-check`: no banned libm symbol, no FMA instruction outside `ep9_math`, no second math crate; 5 targets; exceptions in `expectations/fp-check-allow.toml` | Every PR |
| Licence check | `cargo deny check licenses` with the root `deny.toml` allow list ([12-governance §1](./12-governance.md#1-licence)) | Every PR touching `Cargo.lock` or a manifest |
| ADR lint | Changes to H-row crates must reference an ADR | Every PR |
| Dependency guards | Plan v1 guards. `cargo tree -i tsrs_core` empty for the root workspace. `ep9_hints` has 0 dependencies. The jitless build has no `jit` feature. `object` crate exact pin ([08-repo](./08-repo.md)) | Every PR |
| Sync | All patches apply on all pins. `sync-check` builds `tsrs_cli` and ep9-typecheck on unix and runs smoke tests (`ep9 check` + hint-export golden file). Windows joins after SP-tsrs-win | Pin or patch change |
| Sanitizers | ASan nightly. TSan from S2. Miri on `ep9_value`, `ep9_numconv`, string algorithms. loom on concurrent GC structures (2 markers + 1 mutator; the card clear/dirty pair). TSan cannot see JIT or AOT code; the verifier gate covers it | Nightly |

---

## 6. Agent workflow

- **Work packages.** One test262 directory, its spec clauses, one `ep9_builtins_*` crate and its slice of the expectations file. Examples: Object (3,411 tests), Array (3,083), String (1,223), Promise (732), RegExp (1,879), TypedArray family, Iterator (654), Date (594), collections, Proxy/Reflect, Atomics/SAB, ERM, Temporal (4,605), intl402 (3,382).
- **Start.** SPEC starts at M1a. It does the M2 gate directories first: Array, String, Object, Function, Math, JSON, Number, RegExp.
- **Rules.** Builtin crates use `#![forbid(unsafe_code)]` and reach the heap only through ObjectOps. Each spec step gets a comment with its text. Merge gate: 0 test262 regressions, GC-stress subset passes, clippy clean, perf gate green. Every agent PR shows its test262 and instruction-count deltas.
- **New AOT packages.** Agents own the hint-soundness corpora, the typed kernel ports, SP-HintPrecision and SP-RegionCoverage, each under an independent reviewer agent (ADR-P1). Each has a strong oracle: node output or V8 checksums.
- **Contract ownership.** `.github/CODEOWNERS` lists the owner on `ep9_value`, `ep9_heap`, `ep9_platform`, `ep9_bytecode`, `ep9_vm`, `ep9_frontend`, `ep9_masm`, `ep9_baseline`, `ep9_opt`, `ep9_unit`, `ep9_aot` and `ep9_facts`. It is advisory ([12-governance §4](./12-governance.md#4-review-and-ownership)). A change there, and every `unsafe` change, gets a Codex adversarial review plus a reviewer agent that did not write it (ADR-P1).
- **Runtime agents** use node/test and WPT as their oracles in the same way.

---

## 7. Risks and mitigations

RK1-RK28 are plan v1 risks R1-R28, re-tagged. RK29-RK44 are new.

| ID | Risk | Effect | Mitigation | Early signal | Evidence |
|---|---|---|---|---|---|
| RK1 | The interpreter misses V8-jitless parity | The JIT sits on a slow base | Perf CI from week 1. M0 canary. ICs in M1. Hard M2 gate | M1 Octane-7 < 1.0x QuickJS | [R07] LibJS JIT removal; Brimstone 0.89x |
| RK2 | One kernel blocks M2 forever | JIT start slips without limit | Parity-debt rule ([09-milestones](./09-milestones.md)). RegExp is outside the M2 geomean, with its own RX1 gate | A kernel < 0.8x at M2 review | [R07] Brimstone 0.60-0.62x on RayTrace and EarleyBoyer, 0.47x on RegExp |
| RK3 | CSS misses a pointer (derived, one-past-end, SIMD) | Use-after-free | Spill trampolines, interior lookup, call-only safepoints, nightly GC-stress, ASan | Any GC-stress diff | [R05] |
| RK4 | A GC reference hides in a Rust container | Use-after-free | Branded `Local`, `ScratchRoots`, lint, forbid(unsafe) builtins | GC-stress failures in builtins | [R05] probe S7 |
| RK5 | oxc API churn | Adapter breaks | Exact pin. One adapter crate. Planned bumps with diffs | Bump work > 1 PM/yr (P2) | [R04] 60 breaking releases a year |
| RK6 | oxc gaps: eval context, depth (incl. non-bracket chains), Function(), offset re-entry, `&str` input | Wrong early errors in eval; abort on hostile input | Upstream track from M0. 3-layer depth guard. Local patches by M1. Shim. P-gates | P1 at M2 exit; any deep-form abort | [R04][R03][P1] |
| RK7 | tsrs re-enters the engine | +6-12 PM and a permanent rebase | Option-B conditions ([01-front-end](./01-front-end.md)); SP-tsrs gives a measured cost. The type oracle stays a separate binary in any case | A D1 re-check (P-gate fail) | [R09][R13] |
| RK8 | tsrs pins leak into the engine graph | oxc drops to Unicode 15.1 | Separate workspaces for `tsrs_cli` and ep9-typecheck. `cargo tree` guard | Guard fails | [R02][R13] |
| RK9 | Heap addresses above 47 bits (LA57, 52-bit arm64, sanitizers) | Crash | Own cage. Startup self-check refuses to run | Cage reservation fails | [R06]; [R07] Boa #4275 |
| RK10 | The non-moving heap fragments | Memory gates fail | S3 evacuation (moves to M5 if G7 fails at M2). MMTk A/B. Nightly memory CI | RSS trend | [R05] |
| RK11 | 8-byte slots cost memory | Up to about 1.75x the heap of a pointer-compressed V8 | G7 caps against both builds | Nightly peak-RSS ratios | [R06]; [P1] node 24.22 without pointer compression |
| RK12 | Data races in concurrent marking | Undefined behavior | Relaxed-atomic rule from M0. TSan, loom, verifier under `--force-tier` and `--force-mode` | TSan or verifier reports | [R05] |
| RK13 | Mid-tier effort overruns | G4 late | Minimal Maglev scope. Reuse IC data. H16/H29 contract and Spike C early. D13 option | M7 progress vs 120-200 PM | [R07] Maglev about 114 PM on existing deopt; [R06] source sizes |
| RK14 | JIT type-confusion bugs | Exploits | W^X, protected dispatch table, tier-diff fuzzing incl. AOT kinds, Fuzzilli, exit counters, security owner | Fuzz mismatch rate | [R07] about 45% of V8 CVEs |
| RK15 | Stable sibling-call TCO is silently lost | Stack growth or slowdown | Disassembly check, argument-count check, deep-loop test, match fallback | CI check fails | [R06][P1] |
| RK16 | ICU4X stalls near 80% of intl402 | G1 Intl miss | icu_experimental, upstream fixes, own code for gaps | intl402 at M3 | [R08]; [R07] QuickJS 99.7% self-contained |
| RK17 | Agent code drifts from the contracts | Speed loss or unsafety | ObjectOps layering, forbid(unsafe), CODEOWNERS, perf ratchet, GC-stress gates, fast-path differential tests | Rising review rejections | [R07] |
| RK18 | Benchmark noise | False gate results | Instruction counts for PR gates. Gate hosts, load guard, paired rounds and the interval verdict for wall-time gates (§3) | Flapping gates | [R03][R15] |
| RK19 | Lone surrogates in eval source | Rare wrong parses | Placeholder shim with literal fix-ups. P4 | Failing cases > 0 | [R01]; [R04] Nova is lossy; [T262] S7.4_A5/A6 |
| RK20 | regress copies Latin-1 input on every exec | Quadratic `/g` loops | Latin-1 cursor in the RX0 fork | RegExp share in profiles | [R08] |
| RK21 | `panic = "abort"` kills an embedder's process | Embedder crash | The facade catches panics and poisons the isolate in unwind builds. Fuzzing | Crash reports | [R09] panic topic |
| RK22 | Runtime or AOT work starves the speed lane | G2-G4 late | Fixed staff split. RT and the AOT lane never gate M2, M5 or M7. M4 engine items have CORE owners and budget | CORE staffing below plan | C1; [09-milestones](./09-milestones.md) |
| RK23 | Spec churn (decorators back to Stage 2.7; import defer and source phase still move) | Rework | Build Stage 4 and stable Stage 3 only | TC39 notes | [R08] |
| RK24 | Windows diverges | CSS, SEH, VirtualAlloc rework | Tier-1 CI from week 1. SP-GC, SP-JIT-mem, SP-Dispatch on Win64 | Windows-only CI failures | [R09] Windows topic |
| RK25 | Win64 passes only 4 register arguments | Dispatch loses the tail call or adds a load per dispatch | ≤ 4 handler arguments (H15). SP-Dispatch on Win64 hardware | Windows > 10% slower than Linux x64 | [P1] |
| RK26 | T2 needs profiles that T0 does not record | M2 gates break later, or G4 misses | H16/H29 ADRs in M0. T0 records the profiles before the M2 gate. Spike C in M1 | Spike C fails its gate | [R06] |
| RK27 | tsrs on Windows is untested upstream | `ep9 check` fails on a tier-1 platform | Unix only until SP-tsrs-win passes; ≤ 2 patches | Spike fails | [TSRS] release.yml:132-133, build.rs |
| RK28 | The agent team delivers CORE work slower than planned (D3) | v1 late | M0-exit throughput check (D3); critical-path table; levers ([09-milestones](./09-milestones.md)) | PM done per week below half the plan rate (est.) | [R07] Maglev staffing |
| RK29 | Overselling AOT | The architecture chases a claim that no measured system meets | D15 = (a); (b) only as gated A5 with mechanism-neutral wording. FV1-FV3, each with a numeric gate and a kill. N10. Every claim names its comparator | A document claims "AOT beats V8" with no FV gate | [R15][R17] |
| RK30 | Dual-mode test-matrix cost (mode x tier x platform) | Slower CI and triage; CORE time lost | One optimizer and one frame for all code kinds. Mode matrix nightly, not per PR. Budgets per stage | The mode-diff triage queue grows two weeks in a row | [R14][R16] |
| RK31 | Naive AOT is slower than existing tiers | Units add size but no speed | K-A3 and K-A4J. M5c generated interpreter as the jitless fallback | baseline-AOT < 1.3x T0 on JITLESS-SET | [R15][R17] Hermes untyped native slower than its own baseline JIT |
| RK32 | An unsound hint is used without a guard | Type confusion; exploitable | HINTED facts always get guards (H30, H32). Never THROW on a hint failure. Hint chaos fuzzing. Soundness corpus | Any hint-chaos output diff | [R12][R13] |
| RK33 | Jitless deopt loops | AOT-jitless code thrashes forever | Per-site 4 failures → SLOWPATH; per-function 16 exits → PIN to baseline-AOT (est.). No THROW | Exit counters reach their budget on JITLESS-SET | [R11] ART "forever deopt" |
| RK34 | Code-size blowup | Per-app payload grows about 10x | D19: PGO hot set for hybrid and optimized-AOT; jitless baseline-AOT for reachable functions, bounded by the A3 gate (text ≤ 8x bytecode). SIZE-REPORT | SIZE-REPORT trend | [R14] baseline and mid-tier native is 6-19x bytecode, TurboFan with inlining 28.6x; [R16] (c) for jitless |
| RK35 | Stale or mismatched units | Crash or wrong results after an engine update | Header versions, CPU features, content hash. Fallback by field: the unit's bytecode or source section; a clean load error when no section is trusted (H35 S3). 100% mutation tests | Any mutation test crashes | [R11][R14] |
| RK36 | Unit loading security | Loading a unit runs native code | Content hash at load, optional signature (D6), loader fuzzing, units are trusted code | Loader fuzz crash | [R14] |
| RK37 | Training-profile lock-in | Steady state slower than JIT-only | Runtime feedback overrides PROFILED (H33). Hybrid re-tier. Staleness tests | FV2 steady state > 2% worse | [R11] Leyden replay 20-40% slower |
| RK38 | tsrs oracle churn | ep9-typecheck breaks on tsrs bumps | ep9-owned schema (H37). Public API. ≤ 2 local tsrs patches. Upkeep budget ≤ 1 PM/yr (K-A4H) | Upkeep > 1 PM/yr | [R13] 6 compile errors vs 15-day-old tsrs |
| RK39 | The span join breaks | Hints lost or attached to the wrong node | TS lowering in BytecodeGen (H3). Decoded-text hash. BOM strip. Bundled input gets no hints. A mismatch drops the file's hints and is counted | Join < 99.5% on HINT-SET | [R13][R17] |
| RK40 | Sound mode conflicts with test262 | Conformance claims break | D17 rule: (d) VERIFIED regions keep JS semantics and stay in test262 mode; (c) is opt-in per module, outside test262 mode, with its own test suite | D17 (c) code in a test262 run | [R10][R12][R18] |
| RK41 | FMA or libm differences | Wrong numeric results | H36: no FMA, one Rust Math (CR), clippy + fp-check, FP differential | Checksum diff | [R10][R15] Porffor nbody |
| RK42 | A4/A5 value depends on T2/T3 quality | AOT gains fall short | SP-TypedBound (K-A5pre), SP-RegionBound (K-A5v-bound). A5 after M9 work | Trusted-mode T2 < 1.15x normal T2 | [R15][R17] |
| RK43 | The AOT lane takes review capacity from CORE, or CORE delays A4 | CORE late or A4 late | AOT never gates M2/M5/M7. AOT slack goes to shared code. The orchestrator gives CORE reviews first | A4 not started when T2 compiles kernels | [09-milestones](./09-milestones.md) |
| RK44 | Hybrid feedback cost | The instrumented re-tier costs size and time with no gain | K-A4P. Instrumented T1 only for hot functions (start threshold 1,000 calls, est.) | FV2 time-to-peak or steady-state miss | [R17] .NET never instruments R2R |

---

## Changes vs plan v1

- New mode dimension: `--force-mode` in the runner, a mode x code-kind matrix in the suites and a mode x tier differential in fuzzing.
- New suites: unit version checks, bytecode determinism, FP differential, hint soundness corpus, profile staleness, size report.
- New benchmark sets (HINT-SET, TYPED-KERNELS, SHORT-RUN, JITLESS-SET, STARTUP-SET, SIZE-REPORT) with a set → gate map. New rule: startup and warm-up gates use wall time.
- FV2 compares with node with a warm compile cache. New: the SHORT-RUN latency metric, the AOT-vs-own-JIT report and two test262 runner builds.
- New fuzz targets (unit loader, .ep9h, .ep9prof, hint chaos, FP) and budgets: 1,000 CPU-h at A3, 5,000 at A4 (est.).
- New CI checks: feature matrix, FMA lint, zero text relocations, `ep9_hints` zero deps, ep9-typecheck in sync-check, unit golden files, determinism.
- Risk IDs renamed RK (v1 R1-R28 → RK1-RK28). RK7, RK22 extended. 16 AOT risks added (RK29-RK44).
- Unchanged: test262 rules, instruction-count perf gates, v1 fuzz targets and budgets, agent rules.
- Decisions 2026-10-20 ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)): the speed-gate rule (PERF-NOISE); barrier checks V1-V3, the model test and 5 negative controls (H11); FP lint = `clippy.toml` + `xtask fp-check` (H36); H35 S3 outcomes in the unit checks; licence check; bench pin names (NAMING); JetStream harness note; K-A5v-cov, K-A5v-bound, K-A5v; RK7, RK18, RK28, RK29, RK35, RK40, RK41, RK42, RK43 updated.
- Amended 2026-10-21 ([decisions-log §7](../decisions-log.md#7-2026-10-21-h15-dispatch-on-x86-64-sp-dispatch-reopen), PERF-NOISE amended): a class S result names its CPU model and holds for that model only (§3 row S).

---

## Evidence tags

| Tag | Path |
|---|---|
| [R01] | [../research/01-tsrs-architecture.md](../research/01-tsrs-architecture.md) |
| [R02] | [../research/02-tsrs-conformance.md](../research/02-tsrs-conformance.md) |
| [R03] | [../research/03-parser-perf-and-features.md](../research/03-parser-perf-and-features.md) |
| [R04] | [../research/04-parser-alternatives.md](../research/04-parser-alternatives.md) |
| [R05] | [../research/05-gc-design.md](../research/05-gc-design.md) |
| [R06] | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R07] | [../research/07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R08] | [../research/08-spec-subsystems.md](../research/08-spec-subsystems.md) |
| [R09] | [../research/09-critic.md](../research/09-critic.md) |
| [R10] | [../research/10-js-aot-precedents.md](../research/10-js-aot-precedents.md) |
| [R11] | [../research/11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R12] | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R13] | [../research/13-tsrs-type-oracle.md](../research/13-tsrs-type-oracle.md) |
| [R14] | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R15] | [../research/15-aot-perf-reality.md](../research/15-aot-perf-reality.md) |
| [R16] | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R17] | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) |
| [R18] | [../research/18-sound-regions.md](../research/18-sound-regions.md) |
| [P1] | Plan v1 local probe (session-local); re-checked by the named M0 spike (SP-Dispatch; M0 week 1 shell check for node pointer compression) |
| [T262] | `third_party/test262` at `test262.commit` |
| [TSRS] | `third_party/tsrs` at `tsrs.commit` |
