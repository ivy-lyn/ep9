# ep9 plan: runtime layer

- The runtime layer keeps its v1 choices: event loop, modules, `node:*`, web APIs, workers, inspector and Node-API. Only the TypeScript and snapshot rows change.
- The code cache, the startup snapshot and bytecode units form A1, the first AOT product. A1 ships no machine code and serves all three modes.
- New engine contracts: unit loading (H35), AOT frames in stack walks (H14), AOT deopt-all (H24), profile export (H16) and TS flags (H3).
- `ep9 run`, `ep9 check` and `ep9c build` cover every mode. `--unit` selects hybrid mode in the JIT build and AOT-jitless mode in the jitless build.
- tsc and prettier must match node in each shipped mode. Startup, RSS and size are reported per mode from A1 (G15).
- eval and `new Function` compile at run time in every mode (H5). The lone-surrogate shim is unchanged.

IDs used here: G13, G15; H1, H2, H3, H5, H7, H13, H14, H16, H24, H31, H32, H33, H35, H36; C12, C13; D2, D4, D5, D16, D17, D19; A1, A2, A3, A4, A5; K-A3, K-A4J, K-A4P, K-A4H; M0-M8, M10, M5c; N1; P4; SP-tsrs-win; STARTUP-SET; VERIFIED; CORE and AOT lanes; JIT mode, AOT-jitless mode, hybrid mode.

Evidence tags: [R01] [R04] [R08] [R09] [R10] [R11] [R12] [R14] [R15] [R16] [R17] [R18] [P1] [T262]. Paths are in the table at the end.

## 1. Components

| Component | Choice | Milestone |
|---|---|---|
| Event loop | tokio 1.54.0, one current-thread runtime per isolate thread. Node phases: timers (own heap, Node order), poll (I/O completions into a ready queue), check (`setImmediate`), close | M4 |
| Job order | After every callback: the nextTick queue, then promise jobs, then ClearKeptObjects, then FinalizationRegistry cleanup [R08] | Hooks M1, loop M4 |
| Modules | ESM (cyclic records, TLA, attributes, JSON/text, defer). CJS loader with `require(esm)`. Resolution: oxc_resolver 11.24.3 with `exports`/`imports` | M4 |
| **TypeScript (changed)** | TS runtime constructs lower in BytecodeGen (H3, [01-front-end.md](./01-front-end.md)). `ep9 check` spawns the tsrs CLI (unix; Windows after SP-tsrs-win). `ep9c build --hints` spawns `ep9-typecheck` | M4; `--hints` in A4 |
| `node:*` modules | M4: process, buffer, events, timers, path, fs, url, util, os, module, assert, console, vm, worker_threads. M6: stream, http/https (hyper 1.12.0 + rustls 0.23.46), net, dns, child_process, crypto subset, zlib | M4, M6 |
| Web APIs | URL (ada-url 4.0.0 or url 2.5.8), TextEncoder/Decoder (encoding_rs 0.8.42, simdutf 0.7.0), AbortController, EventTarget, structuredClone (M4). fetch and streams (M6) | M4, M6 |
| Workers | One isolate per thread. MessagePort. ValueSerializer. SAB backing stores are shared | M4 |
| Inspector | Own CDP server over WebSocket. Runtime + Debugger in M4; Profiler + HeapProfiler in M6. 4-8 PM. No Rust CDP server exists to reuse [R08]. The interpreter debugger hooks have a CORE owner | M4, M6 |
| **Snapshots + code cache (changed)** | **A1: the first AOT product** (§3). Startup snapshot, per-module code cache and `ep9c build --bytecode` units. CORE owns the serializer (2-4 PM [R08]); the AOT lane owns packaging (1-2 PM) | M4 |
| Node-API | `ep9_napi` (D4), from C13. Bun (JSC) shows that Node-API works on an engine that is not V8. Deno shows it can be built in Rust outside Node [R09]. Stack walks cover AOT frames | M6 |
| WebAssembly | D5, decided before M4 starts. Default: wasmtime JS API after v1. vite compiles WebAssembly when it loads [P1] | after M8 by default |

## 2. Engine contracts the runtime needs early

| Runtime need | Engine contract | Fixed in | Built in |
|---|---|---|---|
| Fast startup | Snapshot-ready heap. Stable indices for roots, intrinsics, atoms, fuses and snapshot root shapes (H13) | M0 rules; index lists M1 | M4 (A1) |
| Code cache | Serializable `CompiledUnit`; deterministic, versioned bytecode ABI (H1, H2) | M0 week 3 | M4 (A1) |
| Microtasks, FinalizationRegistry | Host job hooks, ClearKeptObjects, HostEnqueueFinalizationRegistryCleanupJob [R08] | M0 | M1 |
| `vm.createContext`, `$262.createRealm` | N realms per isolate | M0 | M0 month 3 (M3-P1 REALMS; the plan said M1) |
| CommonJS wrapper | `compile_function_ctor(params, body)` | M0 | M0 month 2 (M2-P3 FNCTOR; the plan said M1) |
| Dynamic import, `import.meta` | HostLoadImportedModule, HostGetImportMetaProperties, HostGetSupportedImportAttributes [R08] | M0 | M1 |
| Unhandled rejections | HostPromiseRejectionTracker | M0 | M1 |
| Ctrl-C, watchdog, termination, heap limit | Thread-safe interrupt request polled at safepoints. TerminateExecution. Near-heap-limit callback | M0 | M1 |
| `Error.stack`, CallSite | Per-frame metadata. UTF-16 columns | M0 | M1 (V8 API in M4) |
| Inspector | Position tables, kept ScopeInfo, debug bytecode copy, deopt-all, async stack records | M0 | M4 |
| Sampling profiler | Frame pointers in Rust, JIT and AOT code. VM frames can be walked from a suspended thread | M0 | M6 |
| Heap snapshot | Parsable heap plus constructor names | M0 | M6 |
| Node-API, embedders | `Global` roots, weak refs with finalizers, external-memory accounting. The heap does not move, so handle scopes are cheap [R09] | M0 | M6 |
| Workers, SAB | Isolate per thread. SAB data off heap in reserved, growable regions [R08] | M0 | M3 |
| Atomics.waitAsync | HostEnqueueTimeoutJob plus cross-thread notify | M1 | M3 |
| WebAssembly (later) | Chained signal handlers. VA reservations that coexist with the cage and the code range (H7) [R09] | M0 rules | per D5 |
| **Unit loading (new)** | Container, header versions, verify, fallback by field: bytecode, source section or clean load error (H35 S3) | Spec v0 by M0 exit | A2 |
| **AOT frames in Error.stack, inspector, profiler (new)** | pc->code registry over JIT code and unit ranges (H14) | API M0 | A2 |
| **Debugger attach (new)** | Deopt-all on attach also lazily deopts AOT frames (H24). A debugger write to a local succeeds only on a T0 frame; a write to a context, module or script binding first pops the DebuggerBindingWrite fuse (H24, H31) [R18] | M0 rules | A2 |
| **Profile export (new)** | `ep9 run --profile-out`; FactSnapshot schema = feedback schema (H16) | Schema frozen M1 exit | A3 |
| **Per-module TS flags (new)** | Flags in the CompiledUnit header and the cache key (H3) | M0 week 3 | M4 |
| **eval and `new Function` in every mode (new)** | Bytecode compiler, T0 and source always linked (H5). 1,791 test262 files use eval or `Function(`; 80 test `toString` [R14] | M0 week 3 | M1 |

## 3. A1: code cache, snapshot and bytecode units

A1 is the first AOT product. It holds bytecode only, as the V8 code cache does [R14][R16].

```
 BUILD                                         RUN (JIT, AOT-jitless or hybrid mode)
 ep9 snapshot build ------> startup snapshot --> isolate start: map, deserialize, rewrite pointers
   intrinsics, JS shims, atoms, fuses,             (GC side: 03-gc.md section 6)
   snapshot root shapes (stable indices, H13)
 ep9c build --bytecode app/ --> app.ep9u ------> ep9 run --unit app.ep9u: map, verify header,
   [header | CompiledUnit bytecode |               finalize(); no parse, no machine code
    source | hint section (optional)]
 ep9 run file (first run) --> code cache ------> later runs: key match -> finalize(); else parse
```

**Cache key.** ep9 reuses an entry only when every field matches.

| Key field | Why |
|---|---|
| Decoded-source hash | Any edit to the source |
| Engine build id | Any engine change can change bytecode meaning |
| Bytecode ABI version (H2) | Format change |
| Goal, strict and TS flags (H3) | `[[Set]]` and `[[Define]]` give different code from one source [R12] |

**Invalidation rules.**

| Change | Effect |
|---|---|
| Source text | The hash differs. ep9 ignores the entry, compiles from source and writes a new entry |
| Engine build id or bytecode ABI version | ep9 rejects every cache entry and bytecode unit, then compiles from source: the source file for a cache entry, the unit's source section for a unit (H35 S3) |
| TS flags, goal or strict flag | The key differs, so it is a separate entry |
| Corrupt file or any mutated header field | Cache entry: clean reject, then the source file. Unit: the H35 S3 outcome of the field (bytecode, source section or clean load error). Never a crash ([10-quality.md](./10-quality.md), unit version checks) |
| New startup snapshot | It is built with the engine. Units reach snapshot objects only through stable indices (H13) |

**Gates (A1, at M4 exit; all est.).**

| Gate | Threshold | Set |
|---|---|---|
| typescript.js (9.1 MB) + 1 call: unit vs the ep9 cold source path | ≥ 2x faster (G13) | STARTUP-SET |
| Start with the snapshot vs without | ≥ 2x faster | STARTUP-SET |
| Warm `tsc` with the code cache vs cold | ≥ 1.5x faster | STARTUP-SET |
| Invalidation tests | 100% pass | Unit version checks |

**What other engines get from bytecode caching** (load 9.1 MB of TypeScript, then 1 call).

| System | Source path | Cached path | Gain | Evidence |
|---|---|---|---|---|
| Hermes `.hbc` | 836 ms (its slow `-lazy` path) | 14.9 ms | 56x, engine-specific | [R10][R17] |
| V8 compile cache (node) | 175.6 ms | 87.8 ms | 2.0x | [R17] |
| bun `--bytecode` | 165 ms | 77.5 ms | 2.1x | [R10][R17] |
| qjsc | 380 ms | 37 ms | 10x | [R10][R17] |

- Expect 2-10x on large bundles. The gain depends on how fast ep9's own oxc-based source path is, so ep9 measures it there [R17].
- App heap snapshots come after v1.

## 4. Commands per mode

```
                    ep9, JIT build (`jit`)                      ep9, jitless build / iOS library
 no --unit          JIT mode: T0 -> T1 -> T2 [-> T3]            T0 only
                    + code cache and snapshot (A1)               + code cache and snapshot (A1)
 --unit app.ep9u    hybrid mode: unit code is the start tier;    AOT-jitless mode: T0 + baseline-AOT
                    hot functions re-tier through                 (+ optimized-AOT if K-A4J passes).
                    instrumented T1 -> T2 (H33).                  Guard failure: SLOWPATH or DEOPT,
                    Only if K-A4P passes; else bytecode only.     then PIN (H32). Never re-tiers.
 every cell:        eval and new Function compile at run time (H5)
```

| Command | Mode or binary | What it does | Stage | Milestone |
|---|---|---|---|---|
| `ep9 run file` | JIT mode (JIT build) | Runs T0 -> T1 -> T2. Reads and writes the code cache | n/a | M4 (cache); tiers M5, M7 |
| `ep9 run --jitless file` | Either build | T0 only, no runtime codegen. Same test262 pass set as the jitless build | n/a | M5 |
| `ep9 run --unit app.ep9u file` | Hybrid (JIT build) or AOT-jitless (jitless build) | Loads the unit. Bytecode always; native code per mode and gate | A1, A3, A4 | M4; A3 gate; M8 |
| `ep9 run --profile-out app.ep9prof file` | JIT build | Writes FactSnapshots, call counts and source hashes (H16) | A3 | With or after M5 |
| `ep9 check` | Spawns `tsrs` | Type check. Unix; Windows after SP-tsrs-win | n/a | M4 |
| `ep9 snapshot build` | Any build | Builds the startup snapshot | A1 | M4 |
| `ep9c build --bytecode` | All modes | `.ep9u` with bytecode and source, no machine code | A1 | M4 |
| `ep9c build --baseline [--profile p] [--all]` | AOT-jitless; hybrid start tier | baseline-AOT. Scope per D19: all reachable functions for jitless, the PGO hot set for hybrid. `--all`: every function | A3 | Gate at M8 exit (M5 exit if D16 = yes) |
| `ep9c build --opt --profile p [--hints [app.ep9h] --tsconfig t]` | AOT-jitless; hybrid | optimized-AOT for the hot set from PROFILED facts. `--hints` adds HINTED facts from a `.ep9h` file, or spawns `ep9-typecheck` to write one | A4 | Gates by M8 exit |
| `--emit obj` or `--emit blob` | ep9c | `.o` for static linking (iOS), or a `.ep9u` blob for mmap (servers) | A2 | Due at M5 start |
| `ep9c inspect app.ep9u` | ep9c | Prints the header, versions, sections and function list | A2 | Due at M5 start |
| `ep9c build --sound` | ep9c; outside test262 mode | Opt-in sound subset. Only if the D17 rule picks (c) at M7 exit. Option (d), automatic VERIFIED regions, needs no flag | A5 | M10 |
| Unit loading from an embedder | `ep9` facade (unstable) | Loads a unit from a file or from memory | A2 | Due at M5 start |

- `ep9c` never compiles eval or `new Function` code. That code compiles at run time (H5).
- Runtime feedback always overrides PROFILED and HINTED facts (H33).
- Optimized-AOT ships in jitless mode only if K-A4J passes. HINTED consumers stay only if K-A4H passes.
- If K-A3 trips, baseline-AOT emit stops. The M5c generated interpreter becomes the jitless top tier if it is ready; else T0 is. Details: [05-aot.md](./05-aot.md).

## 5. API boundary (C12, unchanged)

- The runtime calls the unstable `ep9` facade. ep9 designs no public API (N1).
- Rust `node:*` hot paths (Buffer, fs reads into ArrayBuffers) may call `ep9_vm` internals behind a `runtime-internal` feature.
- New: the facade also loads units. It is unstable like the rest of the facade.
- ep9 requires the default FP environment: round-to-nearest-even, FTZ and DAZ off, FP exceptions masked. An embedder must not change it on a thread that runs JS. Debug builds assert it at Isolate entry (H36-fpenv).

## 6. Runtime acceptance (real programs)

Definitions. They are fixed so that no gate can be met by choosing the sample.
- **node/test subset.** All `test-<module>-*.js` files in node/test/parallel for the modules in the M4 or M6 list (§1), at a pinned node tag. Exclusions live in one ADR list. Each entry names one D4 reason: V8 internals or V8 flags; a feature outside D4, D5 or N1/N2; a platform outside D2. The orchestrator approves the list at M4 start.
- **npm corpus.** The top 1,000 packages from a pinned download ranking. `expectations/` records the source and date.
  - "Pure-JS" means the tarball has no `.node` file and no `.wasm` file.
  - "Loads" means `require()` and `import()` of the main or `exports` entry both finish with no error. They give the same export names as node.

| Check | Modes | Milestone |
|---|---|---|
| typescript@6.0.3 `tsc` builds a reference project. Output equals node | JIT from M4; AOT-jitless from A3; hybrid from A4 (if it ships) | M4; A3 gate; M8 |
| prettier formats a 100-file corpus. Output equals node | Same as tsc | M4; A3 gate; M8 |
| `ep9 -e 0` cold start ≤ 1.0x `node -e 0` (est.) | JIT mode | M4 |
| node/test subset ≥ 80% (M4), ≥ 90% (M6) | JIT mode | M4, M6 |
| WPT url and encoding ≥ 90% (M4). streams and fetch ≥ 90% (M6) | JIT mode | M4, M6 |
| oxc-parser, rolldown and lightningcss napi-rs packages pass their own JS test suites, with the same results as node | JIT mode | M6 |
| App build gate, depends on D5. **Wasm after v1 (default):** a pinned app with no `.wasm` builds with output equal to node; candidate: eslint on a corpus, checked in M4. **Wasm in v1:** `vite build` of a template app [P1] | JIT mode | M6 |
| npm top-1,000 pure-JS packages: ≥ 95% load (est.) | JIT mode | M6 |
| Startup, peak RSS and size report (G15) | Every shipped mode | From A1 (M4) |

- **Hello-world startup is a runtime effect, not an AOT effect.** QuickJS runs from source in 1.99 ms, bun (a JIT) in 5.77 ms and node in 23.5 ms [R15][R17].
  - So `ep9 -e 0` stays a runtime gate. A Node-compatible ep9 pays its own bootstrap in every mode [R17].
- **Memory is the same story.** Hermes JIT RSS (8.8-16.1 MB) equals Static Hermes native RSS (9.8-17.0 MB) [R17]. ep9 measures RSS per mode itself.
- **G15 report.** Binary size per build config. Unit size and native-to-bytecode ratio per mode. STARTUP-SET times and peak RSS per mode. Per-app payload vs total shipped size. There is no size target before the A3 review ([05-aot.md](./05-aot.md)).
- test262 equality per mode and code kind is an engine gate; [10-quality.md](./10-quality.md) owns it.

## 7. Eval source with lone surrogates (unchanged)

- JS strings may hold lone surrogates. oxc needs `&str` [R01][R04].
- Lone surrogates are legal in comments and in string, template and regex literals. test262 S7.4_A5 and S7.4_A6 eval every code unit 0x0000-0xFFFF inside `//` and `/* */` comments [T262]. Elsewhere a lone surrogate is a SyntaxError.
- The `ep9_frontend` shim does one parse plus fix-ups:
  1. Replace each lone surrogate with U+FFFD. It has the same width in UTF-8 (3 bytes) and UTF-16 (1 unit), so offsets do not move. Record the positions.
  2. Parse. oxc rejects U+FFFD outside comments and literals, so those cases stay SyntaxErrors.
  3. Inside comments: nothing more to do.
  4. Inside string and template literals: patch the cooked and raw values with the real code units. This includes template raw strings (`String.raw`).
  5. Inside regex literals: validate the pattern again with each lone surrogate written as `\uXXXX`. Range-order errors then stay correct. The matcher compiles from the original WTF-16 text.
- `Function.prototype.toString` reads the original external string.
- M0 counts the affected test262 cases, incl. S7.4_A5 and S7.4_A6. Any failure trips gate P4.
- **Every mode (new).** eval and `new Function` source reaches this shim in JIT, AOT-jitless and hybrid mode alike. Units never hold eval code (H5).

## 8. iOS packaging (only if D16 = yes)

D16 is **no** (decided 2026-10-20, by rule): no iOS before v1, and the A3 gate stays at M8 exit. iOS limits the address space, so an iOS build needs a cage of about 4 GiB (est.). The jitless build still runs on the 4 tier-1 targets (D2).

- Third-party iOS apps cannot JIT. Only alternative browser engines (EU, Japan) get JIT through BrowserEngineKit [R11][R14].
- So iOS code must be linked into the signed app as a static object. Static Hermes, the V8 embedded blob and Dart AOT do this [R14].

| Item | Plan if D16 = yes |
|---|---|
| Engine | Jitless build (no `jit` feature) as a static library, without `ep9_cli` |
| Units | `ep9c build ... --emit obj`. The `.o` files link into the signed app. No code is mapped at run time |
| Registration | Units register at startup from a static table: pc->code ranges, unwind info and RW data (H14, H35) |
| eval, `new Function` | T0 runs them (H5). No JIT is needed |
| CI | iOS simulator CI from M5 (est.) |
| Cost and timing | +2-4 PM (est.) in A3. The A3 gate moves to M5 exit (D2, D16) |
| Open question | App Store size and launch limits for several MB of units plus a 2-4 MB runtime and compiler. A bytecode-only iOS build may suit many apps [R14] |

## Changes vs plan v1

| Area | v1 | v2 |
|---|---|---|
| TypeScript row | oxc_transformer for enums etc.; `ep9 check` spawns tsrs | Lowering in BytecodeGen (H3); `ep9 check` unchanged; `ep9c --hints` spawns `ep9-typecheck` |
| Snapshot + code cache row | Runtime item, M4 | A1: the first AOT product; adds `ep9c build --bytecode` units; CORE serializer + AOT packaging |
| Engine contracts | 16 rows | + unit loading, AOT frames in stack walks, deopt-all for AOT frames, profile export, TS flags, eval in every mode |
| A1 detail | One table row | Cache key, invalidation rules, gates (incl. G13), precedent numbers |
| CLI | `ep9` commands | Per-mode table for `ep9` flags and `ep9c` |
| Acceptance | JIT-only rows | tsc and prettier per shipped mode; G15 report; `ep9 -e 0` kept as a runtime gate |
| iOS | Only via D11 | Conditional packaging section (D16) |
| Unchanged | | Event loop, modules, `node:*`, web APIs, workers, inspector, Node-API, API boundary, lone-surrogate shim |
| As built 2026-10-26 (M3-P4 DOCS) | | §2 "Built in": N realms per isolate (`$262.createRealm`; the engine side of `vm.createContext`) were built in M0 month 3 (M3-P1 REALMS), and `compile_function_ctor` in M0 month 2 (M2-P3 FNCTOR), not in M1. No contract or milestone gate changes ([decisions-log §12-§13](../decisions-log.md)) |
| Decisions 2026-10-20 | | Unit fallback by field (H35 S3); debugger write rule (H24, [R18]); `--sound` only by the D17 rule; D4 exclusion rule; D16 = no; FP environment rule (H36). See [decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction) |

## Evidence tags

| Tag | Path |
|---|---|
| [R01] | [../research/01-tsrs-architecture.md](../research/01-tsrs-architecture.md) |
| [R04] | [../research/04-parser-alternatives.md](../research/04-parser-alternatives.md) |
| [R08] | [../research/08-spec-subsystems.md](../research/08-spec-subsystems.md) |
| [R09] | [../research/09-critic.md](../research/09-critic.md) |
| [R10] | [../research/10-js-aot-precedents.md](../research/10-js-aot-precedents.md) |
| [R11] | [../research/11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R12] | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R14] | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R15] | [../research/15-aot-perf-reality.md](../research/15-aot-perf-reality.md) |
| [R16] | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R17] | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) (wins over [R16]) |
| [R18] | [../research/18-sound-regions.md](../research/18-sound-regions.md) |
| [P1] | Plan v1 local probe (session-local); re-checked by the D5 review |
| [T262] | `third_party/test262` at `test262.commit` |
