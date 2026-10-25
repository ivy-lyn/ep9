# ep9 plan: milestones, lanes and staffing

- Four lanes run in parallel: CORE (VM, GC, JIT), SPEC (compliance agents), RT (runtime) and the new AOT lane. CORE is the critical path.
- M2 stays the hard gate. No JIT tier that runs JS starts before T0 reaches V8-jitless parity.
- The dual-mode contracts (A0) cost about 1-2 PM (est.) inside M0 and M1. After M1 exit, no H row is rewritten (G8, G11).
- The AOT lane builds A1h and A2 after M1 exit, A1 in M4, A3 with M5 and A4 with M7. It never gates M2, M5 or M7. It gates the M0 and M1 exits only through two time-boxed A0 spikes.
- v1 (M8) ships JIT mode, A1 artifacts and AOT-jitless mode. Baseline-AOT, optimized-AOT, hybrid mode and HINTED facts each ship only if their kill rule passes (K-A3, K-A4J, K-A4P, K-A4H).
- Team (D3): AI agents led by an orchestrator agent; the owner reviews at will. Total M0-M8 effort is about 285-524 PM (est.), a size of work. The calendar months in §6 are human-team figures and do not apply.

IDs used here: lanes CORE, SPEC, RT, AOT; M0-M10, M1a, M5c; A0, A1h, A1, A2, A3, A4, A5; H1-H37; G1-G15; D1-D21; SP-GC, SP-VA, SP-Value, SP-Dispatch, SP-JIT-mem, Spike B, SP-Unit, Spike C, SP-Facts, SP-tsrs-win, SP-tsrs, SP-HintPrecision, SP-TypedBound, SP-RegionCoverage, SP-RegionBound; FV1-FV3; VERIFIED; K-A3, K-A4J, K-A4P, K-A4H, K-A5pre, K-A5, K-A5v-cov, K-A5v-bound, K-A5v; T0-T3; S0-S4; RX0-RX2; P1-P5; L1; HINT-SET, TYPED-KERNELS, SHORT-RUN, JITLESS-SET, STARTUP-SET, SIZE-REPORT; RK22, RK28, RK43.

Evidence tags: [R05] [R06] [R07] [R08] [R09] [R10] [R11] [R12] [R13] [R14] [R15] [R16] [R17] [R18] [P1] [T262] [TSRS]. Paths are in the table at the end.

PM means senior human person-months. "(est.)" marks a number that no research report measured. The team checks each one again at its first milestone review. Under D3 it is a size of work only; no calendar is derived from it.

---

## 1. Lanes

```
       M0          M1            M2            M4            M5           M7             M8          optional
CORE  [M0]------->[M1]--------->[M2]==gate===>(M4 items)--->[M5]-------->[M7]---------->[M8 v1]--->[M9]
3.5   contracts   perf slice    interpreter   snapshot,     T1 + S2b     T2 + facts     hardening,
FTE   +A0 part 1  +Spike C      parity        L1, debug     + RX2        +SP-TypedBound  mode gates
                  +SP-Facts     (hard gate)   hooks
                  +A0 part 2
SPEC             [M1a]------------------------>[M3 compliance]  (closes after M2; M2 gates stay green)
0.5
RT               [M1]-->[M4 runtime alpha + A1]-------------->[M6 runtime beta]----------->[M8 share]
1.0
AOT   A0 part 1 -> A0 part 2 -> A1h + A2 -----> A1 ---------> A3 ---------> A4 ---------> gates ---> [A5=M10]
1.0                            (after M1 exit) (in M4)       (with M5)     (with M7)      A3/A4 at M8 exit
```

Rules:
- No JIT tier code runs JS before the M2 gate. Spike B, Spike C, SP-JIT-mem, SP-Unit and SP-Facts are throwaway code.
- After M1 exit, two kinds of work may start: tier-agnostic JIT infra behind `jit`, and unit machinery (A2) behind `aot`. Neither contains a tier that runs JS.
- In M0 and M1 the AOT senior also works on CORE contract items: H2, H14 and Spike B.
- The AOT lane never gates M2, M5 or M7 (RK22, RK43). A4 needs T2, so the A4 gates fall at M8 exit.
- The AOT lane does gate the M0 and M1 exits, through SP-Unit and SP-Facts (A0). Each spike has a 2-week time box (est.) beyond its budget (§4).
- A3 is built with T1 in M5. Its gate is at M8 exit, or at M5 exit if D16 = yes. A K-A3 pre-check runs at M5 exit in every case. It does not gate M5.
- SPEC counts as done only while every M2 gate stays green.

---

## 2. Milestones

### 2.1 Overview

| M | Name | Lane | Effort PM (est.) | Depends on |
|---|---|---|---|---|
| M0 | Contracts, spikes, skeleton + A0 part 1 | all | 12.5-19 | none |
| M1 | Perf vertical slice + A0 part 2; checkpoint M1a | CORE (+AOT) | 12.5-22 (incl. Spike C 1.5-3, SP-Facts 0.25-0.5) | M0 |
| M2 | Interpreter parity: hard gate before any JIT tier code | CORE | 14-28 (incl. RX1 4-8, which may run into M5) | M1 |
| M3 | Compliance complete (agents) | SPEC | 22-42 | M1a; closes after M2 |
| M4 | Runtime alpha + A1; A1h due | RT (+CORE 4-8) | 18-36 (+AOT lane 2-4) | M1; parts of M3; D5 decided |
| M5 | Baseline JIT + GC S2b; A2 due at M5 start | CORE | 26-52 (T1 18-36, S2b 6-12, RX2 2-4) | M2 |
| M6 | Runtime beta | RT | 14-28 | M4 |
| M7 | Speculative mid tier + fact inputs; A4 build; SP-TypedBound | CORE (+AOT) | 120-200 (+AOT lane A4 16-32) | M5 exit (S2b functional) |
| M8 | v1 hardening and release; A3 and A4 gates | all | 18-40 (+Wasm 3-6, sandbox 6-12, PTC 2-4 if chosen) | M3, M6, M7 |
| M9 | Top tier (optional); serves JIT and optimized-AOT emit | CORE | 120-360+ | M7 |
| M10 | Sound typed code (optional) = A5; by the D17 rule at M7 exit | AOT | (c) 12-24 prototype; (d) 16-33 | M7; benefits from M9 |

### 2.2 Scope and exit criteria

All exit items of a milestone must hold. All 4 targets are tier-1 from week 1, Windows x64 included [R09]. From M2 on, speed gates hold on macOS arm64 and Linux x64, and Windows x64 stays within 10% of Linux x64 (est.). Every speed gate and spike timing follows the [10-quality](./10-quality.md) §3 speed-gate rule. Results from other hosts are provisional.

**M0. Contracts, spikes, skeleton + A0 part 1**
- Scope (v1): workspace, CI on 4 targets, perf CI, test262 runner. Value, cage, header, GC S0. Bytecode v1, VM stack, frames, limits. Dispatch DSL with 3 expansions and ≤ 4 handler arguments. Front end F1, depth layers 2-3, `CompiledUnit`. Shapes v0, named-property IC v0, strings v0. H16 slot kinds and the H29 ADR draft. All M0 spikes, incl. SP-tsrs-win. Builtin ABI frozen (H20).
- Scope (A0 part 1): ADRs for H2, H3 header fields, H5, H7, H8, H13 rules, H14, H27, H35 container spec v0 and H36. Determinism CI. SP-Unit after Spike B, on placement (b) (§4).
- Exit (how it is checked, and the status after M3-P4: §8, "M0 exit check and status"):
  1. Full test262 run (102,994 runs at d53ebc8b) ≤ 10 min. Expectations committed per target.
  2. Parse negatives 4,653/4,653 and positives ≥ 99.98% on all 4 targets.
  3. `language/` ≥ 40%.
  4. richards and deltablue run correctly, each ≥ 0.6x QuickJS (design canary).
  5. Spikes green: SP-GC, SP-VA, SP-Value, SP-Dispatch, SP-JIT-mem, Spike B (gates in §4). SP-tsrs-win result recorded.
  6. GC-stress passes every passing test.
  7. Bracket, binary, member, call, comma, template and class nesting: cap−1 compiles, cap+1 gives RangeError, the process stays alive. `throw new Error()` prints file:line:col with a UTF-16 column.
  8. Static asserts: Value, JsResult and word0 are 8 B.
  9. Every H row marked "Fix by M0" is merged as an ADR. Builtin ABI frozen.
  10. A0 part 1 ADRs merged: H2, H3 header fields, H5, H7, H8, H13 rules, H14, H27, H35 container spec v0 with header struct and golden test, H36.
  11. SP-Unit green on 4 targets.
  12. Identical bytecode hashes across two builds per target on the test262 + bench corpus.

**M1. Perf vertical slice + A0 part 2 (checkpoint M1a)**
- Scope (v1): full shapes (transitions, slack, constness, watchpoints, dictionary, validity cells). Elements kinds. CacheIR-like ICs for get/set/keyed/call/binary/compare. Megamorphic stub cache. T0 records every H16 slot kind and H29 profile. Closures with register allocation. Strings v1 (ropes, slices, thin, atoms). Generators and async. Realms and host hooks. Proxy and Reflect core ([06](./06-object-model-and-spec.md) §5): every internal method, with and without traps. M2 exit 5 needs them (built-ins/JSON 19/166, Function 14/509 and Object 51/3,411 tests use Proxy; Math 37/327 use Reflect), and the JetStream harness puts a Proxy into every workload realm. Builtins for v8-v7. RX0 regress fork. GC S1. Transition-protocol ADR (H21). oxc eval context, Function() and the depth counter for every recursive production. Fuzzilli REPRL. Spike C.
- Scope (A0 part 2): H16 frozen as the FactSnapshot schema. H29-H34 frozen. Fuse registry built in M1-M2 (H31). SP-Facts inside Spike C.
- M1a checkpoint: H17-H19 frozen, then the SPEC lane starts.
- Exit:
  1. All 8 v8-v7 kernels and Octane-7 run correctly.
  2. Octane-7 ≥ 1.0x QuickJS. v8-v7 7-kernel geomean ≥ 0.75x node --jitless.
  3. `language/` ≥ 85%.
  4. Same pass set under GC-stress with S1 on. Verifier clean.
  5. Fuzzilli 100 CPU-h and front-end cargo-fuzz 24 CPU-h, 0 unique crashes.
  6. No H-row change without an approved ADR amendment.
  7. Spike C gate green. H2, H14 and H16 frozen (H17-H19 froze at M1a).
  8. Conditional, assignment, unary, arrow and `**` chains also pass the cap−1 / cap+1 test.
  9. H29-H34 frozen.
  10. SP-Facts green: the Spike C IR uses the fact lattice and ref nodes. Its deopt metadata is position-independent. One kernel compiled from a PROFILED FactSnapshot file gives identical output at ≥ 90% of live-feedback speed (est.). After the time box, a speed-only miss is recorded, not blocking (§4).

**M2. Interpreter parity (hard gate; unchanged)**
- Scope: dispatch tuning. Wider IC coverage (proto chains, accessors, globals, arrays, strings). Human fast paths for hot builtins, each with a differential test. RX1 irregexp bytecode port starts. GC S2a. Allocation fast path.
- Exit:
  1. Octane-7 ≥ 1.33x QuickJS.
  2. v8-v7 7-kernel geomean (no RegExp) ≥ 1.0x node --jitless. Each of the 7 ≥ 0.8x, or it becomes parity debt (§5).
  3. TS workload ≤ 2.8 s.
  4. Both G7 memory caps hold.
  5. `language/` ≥ 95%, `annexB/` ≥ 90%. built-ins/{Array, String, Object, Function, Math, JSON, Number, RegExp} each ≥ 95% (est.). Every human fast path matches its spec-layer path: 0 mismatches.
  6. Splay ≥ 1.0x node --jitless.
  7. The JetStream 3 M2 list runs. Its ratio to node --jitless is recorded. List rule: the M7 list minus workloads whose code uses a feature scheduled after M2: TypedArray/ArrayBuffer/DataView, BigInt, SAB/Atomics, Intl, Temporal and dynamic import (M3), Worker (M4), WebAssembly (M8). Proxy, Reflect and WeakRef/FinReg are M1 or earlier (06 §5, 03 §5), so they do not remove a workload. 34 of 63 workloads at the current pin (`expectations/jetstream-m2.txt`, regenerated by `tools/bench/jetstream/lists.mjs`).
  8. Fuzzilli 500 CPU-h, 0 unique crashes.
  9. P1, P2 and P4 checked.
  10. Gates 1-3 hold with all H16/H29 profiling on.

**M3. Compliance complete (agents; unchanged)**
- Scope: all ES2026 and ES2027-so-far builtins. Annex B. Modules (rest). Proxy and Reflect (rest of their test262 directories; core in M1). TypedArrays. SAB, Atomics, agents. WeakRef/FinReg (rest; core with the GC from S0, 03 §5 and 06 §5). ERM. Iterator helpers. Intl. Temporal. Date. BigInt (dashu).
- Exit:
  1. test262 full suite ≥ 97.5% per file, by the G1 rule (V8: 97.69%).
  2. `intl402` ≥ 90%. Temporal (6,715 tagged tests) ≥ 97%.
  3. Every M2 gate still green.
  4. Nightly GC-stress test262: 0 new failures.
  5. Fuzzilli 2,000 CPU-h, 0 unique crashes.
  6. All lone-surrogate eval cases pass, incl. S7.4_A5/A6 [T262].

**M4. Runtime alpha + A1 (A1h due)**
- Scope (v1): event loop, ESM + CJS, resolver, `node:*` M4 set, web APIs M4 set, workers. CDP Runtime + Debugger. V8 stack API. `ep9 check`. CORE-owned engine items: snapshot serializer, interpreter debugger hooks, L1 lazy compile.
- Scope (new): TS lowering in BytecodeGen with per-module flags (H3). A1: code cache, startup snapshot with H13 stable indices, `ep9c build --bytecode` (bytecode + source, no machine code). A1h due: ep9-typecheck, ep9_hints schema v0, hint join, SP-HintPrecision.
- Exit:
  1. `ep9 -e 0` cold start ≤ 1.0x `node -e 0` (est.).
  2. Snapshot start ≥ 2x faster than without (est.). Warm `tsc` with code cache ≥ 1.5x faster than cold (est.).
  3. tsc and prettier gates ([07-runtime](./07-runtime.md)).
  4. node/test subset ≥ 80%. WPT url and encoding ≥ 90%.
  5. A scripted CDP session (attach, breakpoint, step in/over/out, scopes, console eval) passes on all 4 targets. The session also runs the R18 debugger probes (B28, B28c, interrupt-poll `Runtime.evaluate`) under `--force-tier` for each tier: a local write lands in a T0 frame or returns a protocol error; no wrong-type read and no crash.
  6. 100k-deep JS recursion gives RangeError. The process stays alive.
  7. P3 and P5 checked.
  8. A1 gates: typescript.js + 1 call loads ≥ 2x faster from a unit than from the ep9 cold source path (est., G13). 100% of invalidation tests pass. A1h: join ≥ 99.5% of core node kinds, and the SP-HintPrecision report exists. The SP-RegionCoverage report exists and K-A5v-cov is applied.

**M5. Baseline JIT + GC S2b (x64 and arm64, all 4 targets)**
- Scope (v1): `ep9_masm`, W^X code space, dispatch table, unwinding (DWARF and SEH), jitdump, inline TLAB allocation. M5a Sparkplug style. M5b inline ICs and int32 paths. OSR. RX1 finish. RX2 native regexp. S2b concurrent marking.
- Scope (new): T1 emits through the H34 relocation kinds. A2 is due at M5 start. A3 (`ep9c build --baseline`) is built on T1.
- K-A3 pre-check (whatever D16 says): when baseline-AOT first runs JITLESS-SET (target: M5 exit), record baseline-AOT vs T0. It does not gate M5. If it is < 1.3x, the D11 rule picks M5c or T0 as the jitless top tier, the D16 rule keeps iOS out, and the AOT lane builds M5c in the M7 window (§3).
- Exit:
  1. M5a: Octane-7 ≥ 2.0x QuickJS, TS workload ≤ 1.8 s. M5b: Octane-7 ≥ 3.5x, TS workload ≤ 1.1 s.
  2. test262 pass set equal under `--force-tier=baseline` and in the jitless build.
  3. Tier-differential fuzzing 1,000 CPU-h, 0 mismatches.
  4. RX1 gate: RegExp kernel ≥ 0.8x node --jitless, in the jitless build. RX2 gate ([04-execution](./04-execution.md)).
  5. S2b "before M7" gate ([03-gc](./03-gc.md)).
  6. All M2 parity debt cleared, measured jitless.
  7. Only if D16 = yes: the A3 gate (§3).

**M6. Runtime beta (unchanged)**
- Scope: Node-API. `node:*` M6 set. fetch and streams. CDP Profiler + HeapProfiler. npm corpus work.
- Exit:
  1. napi-rs suites, the D5-dependent app gate and npm top-1,000 ([07-runtime](./07-runtime.md)).
  2. node/test subset ≥ 90%. WPT streams and fetch ≥ 90%.
  3. `.cpuprofile` and `.heapsnapshot` load in Chrome DevTools.
  4. Fuzzilli + cargo-fuzz on runtime parsers (JSON, ValueSerializer, snapshot loader): 0 open crashes.

**M7. Speculative mid tier + fact inputs (A4 build)**
- Scope (v1): CFG SSA IR, feedback snapshot, off-thread compile, representation selection, shape guards, elements specialization, small inlining, linear-scan register allocation, eager + lazy deopt, OSR, watchpoints, exit counters.
- Scope (new): one T2 graph builder reads RUNTIME, PROFILED and HINTED FactSnapshots (H30). The AOT lane builds A4 on it. SP-TypedBound runs on TYPED-KERNELS.
- Exit:
  1. Octane-7 ≥ 10x QuickJS (aim 14x).
  2. TS workload ≤ 0.6 s.
  3. JetStream 3 M7 list (no intl, no WorkerTests workloads) ≥ 0.75 × the node --max-opt=2 / node full ratio on the same list (est.).
  4. test262 pass set equal under `--force-tier=opt`.
  5. 10,000 CPU-h of tier-differential and deopt fuzzing, 0 open bugs.
  6. test262 pass set equal when T2 compiles from PROFILED FactSnapshots.
  7. SP-TypedBound and SP-RegionBound reported (inputs to the D17 rule: K-A5pre, K-A5v-bound).

**M8. v1 hardening and release**
- Scope (v1): GC S3 (in M5 if a G7 cap failed at M2). S2b pause and TSan gates. V8 BigInt port. Security response process. Items chosen in D4-D7.
- Scope (new): A3 gate (default timing). A4 gates and kill decisions. Mode acceptance. G15 report.
- Exit:
  1. test262 ≥ 98.5% per file in every tier. `intl402` ≥ 95%.
  2. S3 gate ([03-gc](./03-gc.md)). Both G7 caps still hold.
  3. S2b "by M8 exit" gates, incl. the verifier under forced AOT modes.
  4. ≥ 50,000 cumulative fuzz CPU-h, no open high-severity bug.
  5. All gates green on every tier-1 platform.
  6. Each decided D item meets its own gate.
  7. A3 gate (FV1; K-A3), unless it already ran at M5 exit.
  8. A4 gates measured (K-A4J, K-A4P, K-A4H). The orchestrator applies the kill rules at the AOT ship review and logs them (D11, D15, D18; [11-decisions](./11-decisions.md) §2). Losers then leave the v1 build.
  9. test262 pass set equal in every shipped mode x code kind (G12).
  10. Mode acceptance: tsc and prettier output equals node in each shipped mode.
  11. G15 report: per-app payload, total shipped size, startup and RSS per mode. It adds the SHORT-RUN latency metric and the AOT-vs-own-JIT speed report ([05-aot](./05-aot.md) §3).

**M9. Top tier (optional).** Own-IR passes (LICM, load elimination, escape analysis) or a Cranelift lowering. It serves JIT emit and optimized-AOT emit. Exit: Octane-7 ≥ 20x QuickJS.

**M10. Sound typed code (optional) = A5.** The D17 rule (M7 exit) picks (d) automatic VERIFIED regions or (c) an opt-in sound subset, or drops M10. (d) stays legal in test262 mode; (c) runs outside it. Exit: FV3 (≥ 1.1x V8 full at peak on ≥ 2 of 4 TYPED-KERNELS and ≥ 1.3x ep9 A4 hinted code) and K-A5v or K-A5.

**v1 (M8) ships:**

```
always:       JIT mode (T0 -> T1 -> T2) + A1 artifacts (code cache, startup snapshot, bytecode units)
always:       AOT-jitless mode (jitless build, T0, A1 units)
conditional:  jitless top tier = baseline-AOT if K-A3 passes; else M5c if ready; else T0 only
conditional:  optimized-AOT only if K-A4J passes
conditional:  hybrid mode only if K-A4P passes;  HINTED facts only if K-A4H passes
not in v1:    T3 (M9), sound typed code (M10/A5)
```

---

## 3. AOT stages A0-A5

Detail and kill rules live in [05-aot](./05-aot.md). Benchmark sets live in [10-quality](./10-quality.md).

| Stage | Inside | Lane | Scope | Gate (est.) | Kill | PM (est.) |
|---|---|---|---|---|---|---|
| A0 Foundations | M0 (part 1), M1 (part 2) | AOT senior + CORE leads | H-row ADRs (M0 and M1 lists above), container spec v0, determinism CI, SP-Unit, SP-Facts | SP-Unit green on 4 targets. SP-Facts: Spike C IR uses fact lattice + ref nodes, PI deopt metadata, one kernel from a PROFILED FactSnapshot file gives identical output at ≥ 90% of live-feedback speed. Identical bytecode hashes across two builds per target | none (contract) | 1-2 |
| A1h Hint oracle v0 | after M1 exit, due M4 exit | AOT | tools/ep9-typecheck builds (unix), ep9_hints schema v0, join in ep9_frontend (no codegen use), SP-HintPrecision report on HINT-SET, SP-RegionCoverage (K-A5v-cov) | join ≥ 99.5% of core node kinds | early D18 off-ramp; K-A5v-cov | 1-3 |
| A1 Bytecode AOT | M4 | CORE (serializer, snapshot 2-4 PM, already in v1 M4) + AOT (packaging) | code cache, startup snapshot, `ep9c build --bytecode` | typescript.js + 1 call loads ≥ 2x faster from a unit than the ep9 cold source path; snapshot start ≥ 2x; warm tsc with cache ≥ 1.5x; 100% invalidation tests | none (the runtime needs it) | 1-2 |
| A2 Unit machinery | after M1 exit, due M5 start | AOT (+CORE JIT lead review) | ep9_masm relocation kinds, ep9_unit writer/loader (.o + .ep9u), link table, RW cells, registry entries, unwind on 4 targets, version fallback, optional embedded stub corpus | a unit with stub calls loads as blob and .o on 4 targets; zero text relocations; 100% of header mutations end in their H35 S3 outcome; exceptions and Error.stack through AOT frames; GC-stress clean with AOT frames | none | 4-8 |
| A3 Baseline-AOT + AOT-jitless mode | built with/after M5 (T1); gate at M8 exit, or M5 exit if D16 = yes | AOT | `ep9c build --baseline`, PGO hot-set selection, jitless load path | FV1: JITLESS-SET ≥ 1.6x node --jitless and ≥ 1.6x T0; test262 equal; TS workload ≤ 1.8 s; unit text ≤ 8x the bytecode of compiled functions | K-A3 | 6-12 (+2-4 iOS if D16 = yes) |
| A4 Optimized-AOT, hybrid, hints | with M7; gates by M8 exit | AOT + CORE T2 owners | T2 AOT emit, guard variants (H32), PROFILED and HINTED inputs, hybrid re-tier (H33) | A4-J: optimized-AOT ≥ 1.5x baseline-AOT on JITLESS-SET. A4-P: the FV2 numbers + test262 equal. A4-H: the K-A4H keep rule | K-A4J, K-A4P, K-A4H | 16-32 (driver + PGO 3-6, T2 AOT emit 6-12, re-tier 2-4, hints 3-6, tests 2-4) |
| A5 Sound typed code (= M10, optional) | after M7 (+ M9 backend for FV3) | AOT | by the D17 rule: (d) automatic VERIFIED regions in `ep9c` AOT units (finder + verifier, fuses, region code + generic sibling), or (c) an opt-in sound subset (`ep9c build --sound`) | FV3; for (d) also K-A5v | K-A5pre, K-A5v-cov, K-A5v-bound, K-A5v, K-A5 | (d) 16-33 (10-21 own); (c) 12-24 prototype |

Why these gate levels (all est.):
- A1: other engines gain 2-10x from cached bytecode on large bundles. Hermes' 56x is engine-specific [R10][R17].
- A3: baseline-class AOT gives 1.6-2.8x its interpreter (Hermes native 1.6-2.0x, weval 2.77x) [R14]. T0 is gated at V8-jitless parity in M2.
- A4: optimized AOT code gives no feedback for free, so hybrid needs an instrumented re-tier (.NET never instruments R2R) [R11][R17]. Guard-removal studies give 2-8%, so hints must prove their gain [R12][R17].
- A5: scriptc ran nbody about 1.15x faster than V8 at peak. C with JS FP rules is about 1.22x, on nbody only [R10][R15][R17].

**M5c fallback (K-A3).** Hermes untyped native gives only 1.3-2.0x its interpreter, so K-A3 can trip [R15][R17].
- Trigger: the K-A3 pre-check at M5 exit (or the A3 gate) shows baseline-AOT < 1.3x T0 on JITLESS-SET.
- Work: M5c, a generated interpreter from the T1 code generator, embedded at engine build time through A2. 3-6 PM (est.).
- Owner: the AOT senior, with CORE JIT lead review, in the M7 window. Stopped A3 emit work partly offsets it (est.). It can delay A4; v1 then waits for the A4 gates (D15 timing t1).
- v1 jitless top tier: baseline-AOT if K-A3 passes; else M5c if it passes test262 equality; else T0 only.

Gate data goes to the orchestrator at the re-check points in [11-decisions](./11-decisions.md) §2. It applies the written rule and logs it. The last point is the M8 AOT ship review.

---

## 4. Spikes

Every spike is throwaway code. A failed gate reopens the named ADRs before the milestone continues.

Time box for the A0 spikes (SP-Unit at M0, SP-Facts at M1):
- Each gets 2 weeks (est.) beyond its budget. This bounds how long the AOT lane can hold the M0 or M1 exit.
- A correctness or structure failure still reopens the named ADRs: load, call, unwind, GC scan, fallback, lattice, ref nodes, PI metadata, identical output.
- If only the SP-Facts speed number (≥ 90%, est.) misses after the time box, the H30 ADR records the result. The exit proceeds, and A4 plans the fix.

| ID | Milestone | Budget PM (est.) | Gate | Owner | Reopens on fail |
|---|---|---|---|---|---|
| SP-GC | M0 wk2 | inside M0 | Spill trampolines for a64, SysV and Win64. Probe cases S1-S7 as unit tests. 0 missed live objects in 10^6 random safepoints per ABI [R05] | GC lead | H10, H9 |
| SP-VA | M0 wk2 | inside M0 | Cage below 2^47 on 4 targets, plus the code range next to it (H7) | GC lead + JIT/platform lead | H7 |
| SP-Value | M0 wk2 | inside M0 | `ep9_value` static asserts: Value, JsResult, word0 = 8 B [P1]. Barrier fast path ≤ 5 instructions on aarch64 plus the cell test (decided form: 4 plus the cell test; `ep9_probe_barrier`) | VM lead | H6, H11 |
| SP-Dispatch | M0 wk2 | inside M0 | DSL on 20 opcodes, 3 expansions. On each target, incl. Win64, over the 3 fixed kernels (loop, branchy, call), the release expansion (H15: sibling calls on every target since the T0 result of 2026-10-21) has geomean time ≤ 1.25x cgoto.c and ≤ 1.5x cgoto.c per kernel. aarch64 also: geomean match time ≥ 1.2x sibling-call time, and sibling call is never slower than match on a kernel (≥ 1.0x) [R06][P1]. x86-64 reports sibling vs match per CPU model, not gated. Match vs cgoto.c is a report on every target. Timing follows the 10-quality §3 speed-gate rule; a hosted x64 result names its CPU model and holds for that model only; hosted CI speed results do not block. 0 non-tail handler exits in release; the spike's x86-64 match loop keeps a copied dispatch jump in each arm (`xtask disasm-check --match-loop`; for T0 a report only). Status 2026-10-25: structural gates and condition 1 pass; condition 2 is open on AMD EPYC 7763 (H15-Q5; V8-A19 branch 3) | VM lead | H15 |
| SP-JIT-mem | M0 wk3 | inside M0 | One generated function per target. Toggle W^X. Call it, unwind and take a backtrace (DWARF CFI; `RtlAddGrowableFunctionTable`) on all 4 targets | JIT/platform lead | H28, H27 |
| Spike B | M0 month 2 | inside M0 | An 11-opcode template JIT (10 planned, plus AddSmi). Keep native-stack JIT frames if one T0→JIT→T0 round trip costs ≤ 18 release-expansion dispatches of the loop kernel on the same host (est.). Result: placement (a) failed in gate window 1; placement (b) passed in gate window 2. x86-64 (b) is built since M3-P0 | JIT/platform lead | H14 and D21 (ADR for one VM stack) |
| SP-Unit | M0 month 3 (M3-P1; moved from month 2 by M2-D21-FAIL) | inside M0; +0.25 | The same 11 opcodes, built on Spike B placement (b) as a unit blob: load, call, unwind, GC-scan and fall back on a version mismatch, on 4 targets. Result: GO on 4 targets (Verdict row 2026-10-25, M3-P2) | AOT senior | H34, H35 |
| Spike C (+SP-Facts) | M1 | 1.5-3; SP-Facts +0.25-0.5 | Small SSA compiler for 2-3 v8-v7 kernels on arm64. One eager deopt from a native frame to a VM-stack frame. Deopt correct under the tier-differential test. Kernels ≥ 3x faster than T0 (est.). SP-Facts: see A0 gate | JIT/platform lead; SP-Facts: AOT senior | H2, H14, H16, H17, H29; SP-Facts: H30-H34 |
| SP-tsrs-win | M0 wk1; again at A1h | 0.25 | `tsrs_cli` builds on x86_64-pc-windows-msvc and passes the `ep9 check` smoke tests with ≤ 2 local patches [TSRS]. At A1h the same check covers ep9-typecheck | Lead + platform lead | none. On fail, `ep9 check` and `ep9c build --hints` print "not supported" on Windows (D2) |
| SP-tsrs (optional) | only if D1 = B is considered | 0.25-0.5 | Apply the minimal fork patch set. Replay 4 weeks of upstream commits. Count conflicts and hours | Lead | none; data for D1 |
| SP-HintPrecision | A1h, due M4 exit | inside A1h | Report per HINT-SET group: any share, class-receiver share, slot-found share, call-target share [R13] | AOT senior + agents | D18 off-ramp: class-receiver and slot-found shares < 5% on ≥ 2 of 3 real-app groups (TS compiler, vscode, hono; [05-aot](./05-aot.md) §5). Then D18 = (c) for HINTED consumers, by the D18 rule |
| SP-TypedBound | M7 | ≤ 1 | T2 with guards removed (test-only trusted mode) vs normal T2 on TYPED-KERNELS | CORE T2 owner + AOT senior | D17 rule: K-A5pre (< 1.15x) → no (c) |
| SP-RegionCoverage | A1h, due M4 exit | ≤ 1 | On the pinned HINT-SET groups, TYPED-KERNELS and one vscode-diff workload: hot-time share in finder-v1 regions (lower of a V8-default and a `node --jitless` profile), in-region member-access share; reported variants: types-off, shipped bundle, allocation sites for tsc's `objectAllocator` types. Reuses [artifacts/sound-regions](../research/artifacts/sound-regions/README.md) on ep9-typecheck output | AOT lead agent | none. Result feeds K-A5v-cov (D17 rule) |
| SP-RegionBound | M7, with SP-TypedBound | +0.25 | Remove guards only inside finder regions, in JIT mode and in jitless mode vs A4 code. Count entry, hoisted interior and aborting checks per region. SIZE-REPORT line: unit text with the second code copy vs the A3 size gate | CORE T2 owner + AOT lead agent | none. Result feeds K-A5v-bound (D17 rule) |

---

## 5. Parity-debt rule (escape valve for M2 gate 2)

1. A kernel among the 7 that is below 0.8x at M2 exit needs a root-cause ADR. The ADR names the missing mechanism.
2. M5 may start only if the 7-kernel geomean, Octane-7, TS-workload and memory gates all pass.
3. The debt must be cleared by M5 exit, measured in the jitless build. If it is not, the orchestrator records the debt in the ADR and applies the M5 parity rule. The owner may override.
4. The RegExp kernel is not parity debt. It has its own RX1 gate, due at M5 exit.

---

## 6. Critical path and staffing

- Calendar months = PM ÷ FTE, with no ramp-up. This is a derived number, not a date. Under D3 (agents) this table is a human-team reference only.
- M0 counts as about 2.5-3.5 months in every team variant. It is bound by spikes and reviews, not headcount (est.).
- Agent compute is not counted. Agent help is already in the SPEC and RT figures [R07].

| Lane | Work in series | PM (est.) | FTE (6 seniors) | Calendar months |
|---|---|---|---|---|
| All | M0 (incl. A0 part 1) | 12.5-19 | 6 | about 2.5-3.5 |
| CORE (VM, GC, JIT) | M1 (without A0 part 2) → M2 → M4 CORE items → M5 → M7 → M8 CORE share | 12-21 + 14-28 + 4-8 + 26-52 + 120-200 + 12-30 = **188-339** | 3.5 (GC, VM, JIT + half of the lead) | **54-97** |
| SPEC (agents + reviewer) | M3 | 22-42 | 0.5 (half of the lead) | 44-84, in parallel |
| RT (runtime) | M4 runtime part → M6 | 14-28 + 14-28 = 28-56 (+D5/D6 items) | 1 (runtime lead) | 28-56, in parallel |
| AOT | A1h → A2 → A1 → A3 → A4 (A0 part 2, 0.5-1 PM, sits inside M1). + M5c 3-6 only if K-A3 trips, partly offset by stopped A3 work (est.) | 1-3 + 4-8 + 1-2 + 6-12 + 16-32 = 28-57 | 1 (AOT senior) | 28-57, inside CORE's window |
| **v1 (M8)** | CORE is the critical path | | | **about 57-100** (+0-6 est. if A4 trails M7) |

Team variants (human-team reference; D3 chose agents on 2026-10-20):

| Seniors | CORE FTE | AOT FTE | v1 calendar (months) | Note |
|---|---|---|---|---|
| 5 | 3.5 (CORE also does AOT) | 0 | about 65-117 | CORE carries 216-395 PM |
| **6 (default)** | 3.5 | 1 | **about 57-100** | lead/front end, GC, VM, JIT/platform, runtime, AOT |
| 9 | 6 | 1.5 | about 34-60 | Maglev had about 6 core people [R07] |
| 13 | 9 | 2 | about 24-41 | above about 6 on M7, the gain is less than linear |

Levers that shorten CORE without new staff:
- Tier-agnostic JIT infra may start after M1 exit (behind `jit`). This shortens M5's calendar, not its PM.
- RX1 has High agent fit ([06-object-model-and-spec](./06-object-model-and-spec.md)). Agents with a reviewer agent can take up to 4-8 PM off CORE.
- D9 (irregexp FFI) or D13 (skip T1) would remove CORE work, at the costs in [11-decisions](./11-decisions.md). Both were decided against on 2026-10-20 (pure Rust; build T1).
- The M4 engine items have named CORE owners: snapshot serializer (2-4 PM [R08]), interpreter debugger hooks, L1. Their 4-8 PM sit on CORE, not RT.
- **AOT slack goes to shared code.** When the AOT senior waits on CORE, the time goes into dual-mode work in `ep9_masm`, `ep9_baseline` and `ep9_opt`. Example: while A4 waits for T2, the AOT senior builds relocation kinds, guard variants and deopt metadata.

```
time after M0 ------------------------------------------------------------------------------>
CORE  (3.5 FTE)   |M1|M2 |M4i|-----M5-----|------------------M7------------------|M8|
AOT   (1 FTE)        |A1h+A2 ..|A1|....A3....|...........A4 (on T2)...........|gates|
                                  slack -> ep9_masm / ep9_baseline / ep9_opt
(order only, not to scale; durations are the ranges in the table above)
```

---

## 7. Effort calibration

v1 calibration [R07]:
- Interpreter at V8-jitless speed: 2-4 PY. Here M1 + M2 = 26-49 PM (A0 part 2 not counted), plus parts of M0.
- ES + Annex B ≥ 97%: 1.5-3 PY with AI help. Intl + Temporal: 0.3-1 PY. Here M3 = 22-42 PM.
- Baseline JIT: 1.5-3 PY. Here M5 = 26-52 PM, incl. S2b and RX2.
- Maglev took about 19 months with about 6 core people (about 114 PM), on top of TurboFan's deoptimizer. ep9 must also build deopt, OSR and watchpoints. Here M7 = 120-200 PM (est.).
- AI agents shrink M3 and the oracle-backed ports most. They shrink JIT and GC work little.

AOT calibration [R14][R16]:
- Static Hermes, Meta's AOT + JIT engine, still has no speculative tier after years of work.
- Dart keeps about 276 AOT branches across about 237k lines of compiler code. Dual mode adds a permanent branch cost.
- HotSpot removed jaotc (JEP 410): "little use" and significant upkeep.
- So the A-stages are estimates with kill paths: A0 1-2 PM + A1h..A4 28-57 PM = 29-59 PM (est.).

Total M0-M8: about **285-524 PM (est.)** = v1 256-465 + AOT 29-59.

---

## 8. M0 first month, week by week

Roles: lead/front end, GC, VM, JIT/platform, runtime, AOT. Agents led by an orchestrator agent fill them (D3).

| Week | Deliverables | Owner | Check at end of week |
|---|---|---|---|
| 1 | Root `Cargo.toml` with `exclude` (third_party, fuzz, tools/ep9-typecheck). `rust-toolchain.toml`. `.gitignore`. CI build + tests on macOS arm64, Linux x64, Linux arm64, Windows x64. `sync-lib.sh`, `sync-tsrs.sh`, `sync-test262.sh` (d53ebc8b), `sync-bench.sh`. **SP-tsrs-win** on the Windows runner. Reference shells pinned: node 24.22, d8 via jsvu (confirm pointer compression), QuickJS 2026-06-04, bun 1.4.3, Brimstone 64d6698. node --max-opt=2 vs node full on JetStream 3 JS workloads. M2 and M7 lists in `expectations/`. Perf CI v0: per-commit instruction counts as JSON. test262 front-matter indexer. Parse-only runner through `ep9_frontend` and oxc `=0.155.0`. ADR template; drafts of H6-H12, H26, H28. **AOT senior:** H35 spec skeleton, D15 framing, STARTUP-SET harness entries. D2, D8, D12: record their re-check data (decided 2026-10-20) | Lead, platform lead, AOT senior, agents | CI green on 4 targets. Parse runner: 4,653/4,653 negatives and the 7 known positive misses on all 4 targets. SP-tsrs-win result and the JetStream ratio recorded |
| 2 | **SP-Value**, **SP-VA**, `ep9_heap` S0 core (pages, bitmaps, bump allocation, LOS, mark-sweep, GC-stress, verifier, zapping), **SP-GC**, **SP-Dispatch** (`xtask` disassembly and argument-count checks on a64 and x64, incl. Windows). oxc track: write the drafts for #11793, #21953 and the Function() API in `docs/upstream/`; the user posts ([12-governance §5](./12-governance.md#5-upstream-contact-oxc-and-other-public-projects)). **AOT senior:** H8 ShapeId rule, H36: `clippy.toml` bans + `xtask fp-check` stub + `fp-check-allow.toml`, H7 code range inside SP-VA | GC lead, VM lead, AOT senior, agents | SP-GC, SP-Value and SP-Dispatch gates (§4). SP-VA reserves cage and code range on 4 targets |
| 3 | `ep9_frontend` F1: explicit source type, bracket pre-scan, iterative AST-depth pass, depth checks, sized-stack worker, `CompiledUnit` v0, `compile_script`/`compile_module`, scope pass v0. Bytecode v1, disassembler, verifier. VM stack frames, calls without Rust recursion, exceptions, stack-limit RangeError. test262 runner v0. **SP-JIT-mem**. Count lone-surrogate eval cases incl. S7.4_A5/A6 (P4 baseline). **AOT senior:** H2 ABI ADR + determinism CI, H14 dispatch kinds + pc→code registry trait, H3 header flags, H5 rule. D1, D7, D14: record their re-check data (decided 2026-10-20) | Lead, VM lead, JIT/platform lead, AOT senior, agents | Full run ≤ 10 min. Nesting forms at cap+1 give RangeError, not an abort. SP-JIT-mem green on 4 targets. Two builds give identical bytecode hashes |
| 4 | Shapes v0, named get/set IC v0, lazy FeedbackVector with H16 slot kinds. Strings v0. Builtin ABI v1 draft (`Cx`, `Args`, `JsResult`, `Local`, `ScratchRoots`, throw, call into JS, ObjectOps v0). 10 template builtins for agents. Lints: no `Vec<Local>`, no `Drop` in heap types. First perf report vs QuickJS and node --jitless. Spike go/no-go review. **AOT senior:** H16 slot kinds as FactSnapshot v0, H35 header struct + golden test, empty `aot` feature builds | All | Instruction counts recorded. H2 and H14-H20 drafted. `aot` feature builds on 4 targets. A failed spike gate reopens its ADR before M0 continues |

**M0, months 2-3.**
- Bytecode for the rest of the language: destructuring, classes with private names, spread, iterators, generators/async, labels, `with`/eval slow paths, Annex B semantics.
- `compile_eval` and `compile_function_ctor`, with workarounds until the oxc fixes land: eval parses the text as a Script with no wrapper and drops only the allowed nodes' errors (`eval.rs` E1); `Function()` uses a three-parse wrapper check (`function_ctor.rs` F1).
- The oxc depth-counter patch: upstream PR first, local patch if needed.
- Realms, `$262`, interrupts, and the minimal intrinsics the harness needs.
- **Spike B (month 2) + SP-Unit (month 3, M3-P1).** Spike B places the JIT frames by the D21 rule (one round trip ≤ 18 release-expansion dispatches of the loop kernel keeps native-stack JIT frames).
- ADRs H4, H13 (rules), H24, H25. Drafts of H29-H34. Freeze the builtin ABI (H20).
- D11 and D15 were decided on 2026-10-20 ([11-decisions](./11-decisions.md)). Run the M0 exit gates.

Status of these items after M3-P4 (2026-10-26; decisions-log §12-§13):

| Item | Status |
|---|---|
| Bytecode for the rest of the language | Built: destructuring, classes with private names, spread, iterators, generators, async functions, async generators and `for await`, labels, `with` and eval slow paths, Annex B functions in sloppy eval code, ES modules with `import()` and top-level await. Bytecode ABI 5 (M3-P0); corpus digest BCG-CORPUS-14 (M3-P4) |
| `compile_eval`, `compile_function_ctor` | Built with the E1 and F1 workarounds. M3-P4 built the 3 eval items that M3-P2-OPEN left open, all inside ABI 5 (M3-P4-EVAL-SCOPE): E6 (`super()` in direct eval code), E9 (a private name of the caller's class in direct eval code) and D3d (a `var` of a direct eval in a parameter list that has the name of a parameter is a SyntaxError, EvalDeclarationInstantiation step 3.d) |
| Realms, `$262`, interrupts, intrinsics | Built: N realms per isolate (REALM cells, the realm switch at cross-realm calls), `$262.createRealm`; the interrupt budget at calls and loop back edges (H23); a promise reaction job runs in the realm of its handler (H22-Q10) |
| Spike B + SP-Unit | Spike B: placement (b) passed in gate window 2; x86-64 (b) built in M3-P0. SP-Unit: GO on 4 targets (M3-P2). §4 and [spike review 2 draft](../measurements/2026-10-25-m0-spike-review-draft.md) |
| ADRs H4, H13, H24, H25; drafts of H29-H34 | H4, H13, H24 and H25 Accepted. H29-H34 are Draft (H29 and H30 need only Draft at M0; H31-H34 freeze at M1 exit) |
| Freeze the builtin ABI (H20) | M3-P3 wrote the H20 Freeze table, the R5 test with two realms and the core golden (a tripwire). M3-P4 wrote review rows 1, 2 (Codex) and 4 of H20. H20 is Draft; row 3 and the Accept and Freeze steps are M3-P5 (plan v8) |
| Run the M0 exit gates | The GATES-2 dry run ran at M3-P3 (below). The exit run is M3-P5 |

**M0 exit check and status** (the exit items are in §2.2):

- How it is checked: `cargo xtask m0-exit` (`tools/xtask/src/m0_exit.rs`) prints PASS, FAIL, OPEN, NOT-RUN or N/A for each item, for one host and for Windows x64 from the jobs of a CI run. It runs no test suite: it reads the outputs of a run (the test262 parse, exec and GC-stress JSON, the corpus and nesting logs with their exit-status lines, the cross-engine run, the spike Verdict tables, the ADR index). CI makes these inputs on the 4 targets: the `test262` job uploads `m0-exit-inputs-<target>`, and the `test262-gc` job runs GC-stress as 6 shards per target (`cargo xtask m0-exit --gc-shard k/6`) and uploads one JSON per shard. Item 4 is N/A on Windows by design (a gate-host speed reading). The exit holds when the tool exits 0 on mac a64, linux a64 and x86-64 Linux, with the Windows column from the CI run of the exit head (plan v8; decisions M3-P3-GATES2-DEV, M3-P3-DRYRUN-CI).
- Status at the M3-P3 head (`63f115a`, 2026-10-25; the GATES-2 dry run, [decisions-log §13.5](../decisions-log.md)): items 1-3, 6-8 and 10-12 PASS in every column (mac a64, linux a64, x86-64 Linux, the Windows CI column). Three items are OPEN (the item 9 text shows the status after M3-P4):
  - item 4 (host columns): perf run 3 passes the canary (richards 1.504x, deltablue 1.636x QuickJS, gate window 3), but it ran at `88dacff`, not this tree. Gate window 4 on the exit tree closes it (M3-P5);
  - item 5: SP-Dispatch condition 2 is open on AMD EPYC 7763 (Zen 3). The orchestrator applied the V8-A19 rule: branch 3, the gate stays (M3-P3-V8A19). Every other spike is GO, and SP-tsrs-win is recorded ([spike review 2 draft](../measurements/2026-10-25-m0-spike-review-draft.md));
  - item 9: at `63f115a`, 26 of 28 rows due at M0 were merged (not H16 and H20). M3-P4 accepted H16, so 27 of 28 are merged and only H20 is open: it is Draft (review rows 1, 2 and 4 written) and not Frozen. 25 ADRs are Accepted.
- Built in M0 months 2-3 ahead of the milestone that names it (decisions-log §12-§13): generators, async functions, async generators and `for await` (M1 scope); N realms per isolate and `$262.createRealm` (M1 "Realms"); ES modules with a loader API, `import()` and top-level await (M3 "Modules (rest)"; JSON modules, import defer and source-phase imports are not built); Date (M3; the legacy Date.parse forms and ECMA-402 `toLocale*String` stay); JSON, Map, Set, WeakMap and WeakSet (the WeakMap ephemeron fixpoint comes in M1 with WeakRef). Also built: the facts exporter `ep9_vm::abi::export_facts` (FeedbackVector to FactSnapshot, H16-Q5; the `.ep9prof` container is H16-Q5b, A3) and `ep9 --facts-out <file>`, which writes the FactSnapshot of the main unit of a run (M3-P4); E6, E9 and D3d in direct eval code (M3-P4, above); and only the scoped embedder forms (`Isolate::with_cx`, `Isolate::with_module_host`; the raw `Isolate::{run_script, call, global, display}` are removed). test262 after M3-P4 (x86-64 Linux sandbox): all files 35,540 / 53,616, `language/` 22,819 / 23,726 (at `63f115a`: 35,323 and 22,607); under GC-stress 35,505 / 53,572 (44 skipped).

---

## Changes vs plan v1

- New AOT lane. Lanes renamed: CORE, SPEC, RT (v1 lanes A, B, C).
- M0 and M1 gain the A0 items, SP-Unit, SP-Facts and the determinism gate. Effort: M0 12-18 → 12.5-19, M1 12-21 → 12.5-22.
- M4 gains A1 and A1h. M5 notes the relocation kinds, A2, the conditional A3 gate and the K-A3 pre-check. M7 gains fact inputs and SP-TypedBound. M8 gains the A3/A4 gates, the AOT ship review, per-mode checks and the G15 report.
- New: the M5c fallback (K-A3) with effort and owner, and the A0 spike time box.
- New optional M10 (= A5).
- Default team: 6 seniors (v1: 5). Staffing variants: 5/6/9/13. Total M0-M8: 285-524 PM (v1: 256-465).
- New spike table. Contract rows renamed to H1-H37. RegExp stages renamed RX0-RX2; the RegExp gate is RX1.
- Unchanged: the M2 hard gate, parity-debt rule, M2/M3/M6 gates, the speed order and the "no JIT tier code before M2" rule.
- Decisions 2026-10-20 ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)): D3 = agents (PM is a size; calendars do not apply); Proxy/Reflect core in M1; M2 JetStream list rule; SP-Dispatch gate by geomean plus per-kernel guards; speed-gate rule; SP-RegionCoverage and SP-RegionBound; M10 by the D17 rule; gate data goes to the orchestrator, not to the user as a question.
- Clarified 2026-10-21 ([decisions-log §7](../decisions-log.md#7-2026-10-21-h15-dispatch-on-x86-64-sp-dispatch-reopen), D21-UNIT): the D21 threshold for Spike B counts release-expansion dispatches of the SP-Dispatch loop kernel on the same host, not "sibling-call dispatches". The number 18 does not change.
- Amended 2026-10-21 ([decisions-log §10](../decisions-log.md#10-2026-10-21-pr-1-review-fixes-ci-gates-and-h15-on-t0), H15-X64-T0, DISPATCH-GATE-T0): §4 SP-Dispatch gates sibling calls against cgoto.c on every target; match vs cgoto.c is a report.
- Clarified 2026-10-22 ([decisions-log §12.9](../decisions-log.md#129-p2-classes-withlookup-eval-front-end-spike-b-infra), M2-P2-D19): Spike B and SP-Unit have 11 opcodes (the planned 10 plus AddSmi), as the [Spike B protocol](../measurements/2026-10-22-spike-b-protocol.md) says. The §4 row, 02 §6 and 04 §5 say 11.
- Clarified 2026-10-22 (decisions-log §12.12 and §12.14: M2-D21-FAIL, M2-P4-W1-D21, M2-P4-SPUNIT-M3, M2-W2-D21-B): placement (a) failed the D21 rule in gate window 1, so SP-Unit moved from M0 month 2 to M0 month 3 (M3-P1) and builds on placement (b). The §4 table has separate Spike B and SP-Unit rows. 02 §6 (H14), 02 §7 and 04 §5 say the same.
- Status added 2026-10-25 (M3-P4 DOCS; decisions-log §13.2-§13.5): §2.2 M0 points to §8; §4 gets the SP-Dispatch status and the Spike B and SP-Unit results; §8 gets a status table for M0 months 2-3, how the M0 exit is checked (`cargo xtask m0-exit`), the exit-item status at the M3-P3 head and the list of items built ahead of their milestone. The A0 part 1 scope says "SP-Unit after Spike B" (it said "inside Spike B"; M2-D21-FAIL). No gate, scope or date changes.
- Status updated 2026-10-26 (M3-P4 DOCS step 2): the §8 status table is "after M3-P4" (E6, E9 and D3d built; BCG-CORPUS-14; H20 review rows 1, 2 and 4); item 9 of the exit status has only H20 open (H16 Accepted, 25 Accepted); the built list names `ep9 --facts-out` and the M3-P4 test262 numbers. The other exit items stay at the GATES-2 dry run; the M3-P5 exit run updates them. No gate, scope or date changes.

---

## Evidence tags

| Tag | Path |
|---|---|
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
| [P1] | Plan v1 local probe (session-local); re-checked by the named M0 spike (SP-Value, SP-Dispatch) |
| [T262] | `third_party/test262` at `test262.commit` |
| [TSRS] | `third_party/tsrs` at `tsrs.commit` |
