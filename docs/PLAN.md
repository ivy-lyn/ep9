# ep9 plan v2: one engine, JIT and AOT

Date: 2026-10-20. This overview replaces the JIT-only plan v1. Detail: [plan/](plan/). Evidence: [research/](research/README.md).

- **One engine, three modes, one optimizer.** JIT mode, AOT-jitless mode and hybrid mode share the bytecode ABI, frames, ICs, deopt metadata, GC and optimizer. The dual-mode contracts cost about 1-2 PM (est.) in M0/M1.
- **Honest AOT line.** The JIT owns peak on general JS. AOT owns startup, warm-up, jitless speed, predictable latency and no JIT compile CPU. "Faster than V8" means only FV1-FV3, each gated.
- **TS types are guarded hints.** `ep9-typecheck` (tsrs, offline) writes them. A4 measures them, and K-A4H removes them if the gain is small. Sound facts exist only in the gated stage A5 (D17 rule: automatic VERIFIED regions or an opt-in subset).
- **Speed order is unchanged.** Contracts (M0) -> T0 parity (M2 hard gate) -> T1 (M5) -> T2 (M7) -> v1 (M8).
- **Parser.** oxc runs behind `CompiledUnit`. tsrs runs only for `ep9 check` and as the offline oracle. It is never in the engine graph.
- **Team (D3).** AI agents led by an orchestrator agent; the owner reviews at will. Effort is in PM as a size of work. Calendar months do not apply.
- **Decisions.** All 21 open items were decided on 2026-10-20 under your instruction (§10).

IDs used here: G1-G15, N1-N13; H1-H37; D1-D21; M0-M10, M1a, M5c; A0-A5, A5v; T0-T3; FV1-FV3; K-A3, K-A4J, K-A4P, K-A4H, K-A5pre, K-A5, K-A5v-cov, K-A5v-bound, K-A5v; SP-RegionCoverage, SP-RegionBound; VERIFIED; V1-V3 (barrier verifiers); P1-P5; S0-S4; RK1-RK44.

## 1. The core idea

You asked: can one engine be optimized for both JIT and AOT? JIT for small size and V8-class peak; AOT for more speed from a TS-typed optimizer.

### 1.1 Short answer

1. Yes: ep9 can be one engine with three modes (JIT, AOT-jitless, hybrid), and the plan makes this an architecture goal from M0.
2. Like Dart, Graal, .NET, ART and HotSpot Leyden, ep9 uses one optimizer for both modes and changes only fact sources and guard policy [R11][R14].
3. No measured JavaScript AOT system beats V8 on general code; the JIT owns peak there, so ep9 makes no general "AOT beats V8" claim [R15][R17].
4. Today's only sound-typed JS AOT (Static Hermes) is 1.5-3.2x slower than V8, but its notes show missing work, not a hard limit [R15][R17].
5. One typed-subset compiler (scriptc) ran nbody about 1.15x faster than V8 at peak, but it refuses eval and changes JS semantics [R10][R17].
6. TypeScript types are unsound by design, so in full-ECMAScript mode ep9 uses them only as guarded hints, never as proof [R12][R13].
7. Nobody has measured what guarded hints buy; guard-removal studies give 2-8%, so stage A4 measures the gain and removes hints if it is small [R12][R17].
8. AOT gives clear value in four places: no warm-up on short runs, speed where JIT is forbidden, predictable latency, and no JIT compile CPU [R15].
9. A representative training profile lets AOT match or beat its own JIT (Graal), but a replayed profile can also lock in slower code [R11][R17].
10. Runtime feedback therefore always overrides a training profile.
11. "JIT for smaller size" holds for the per-app payload (bytecode is 6-19x smaller than baseline and mid-tier native code), not always for total shipped size [R14][R17].
12. Hello-world startup and memory gaps come from runtime bootstrap, not from AOT, so ep9 measures size, startup and memory in its own runtime [R17].
13. "Faster than V8" is credible in three cases only, each with a numeric gate and a kill criterion.
14. The cases are: against V8 --jitless where JIT is forbidden, short runs before V8 warms up, and sound typed numeric class code (A5, TYPED-KERNELS only).

- **JIT speed in v1.** T2 targets 10-14x QuickJS on Octane-7, about 39-55% of V8 full. On the TS workload it targets about V8 speed (≤ 0.6 vs 0.55 s). Near-V8 Octane peak needs the optional T3 (M9): ≥ 20x QuickJS, about 78% [R06].
- **Where hints can pay.** JIT mode: cold sites and fused facts. Hybrid mode: warm-up and fused facts. AOT-jitless mode: every site. On a conflict, runtime feedback wins (H30 merge rule).

### 1.2 What "faster than V8" means (all gates est.)

| Case | Comparator | Stage | Gate | Evidence | Kill |
|---|---|---|---|---|---|
| FV1 jitless | node --jitless; ep9 T0 | A3; gate at M8 exit (M5 exit if D16 = yes) | JITLESS-SET ≥ 1.6x node --jitless and ≥ 1.6x T0 | Baseline-class AOT gives 1.6-2.8x its interpreter [R14] | K-A3 |
| FV2 short runs | node with a warm compile cache (cold node reported too); ep9 JIT-only with code cache | A4 hybrid; gate by M8 exit | SHORT-RUN wall ≤ 0.9x cached node; time-to-peak ≥ 15% better; steady state within 2%; needs G4 | tsc warm-up 26-30% of 1.6 s; 1.58 s compile CPU [R15]; Maglev-class = V8 on TS workload [R06]; node's cache alone 2.0x on typescript.js load [R17] | K-A4P |
| FV3 sound typed | V8 full; ep9 A4 hinted code | A5 = M10; needs a D17 build choice ((c) or (d)) and a T3-class backend | ≥ 1.1x V8 full on ≥ 2 of 4 TYPED-KERNELS; ≥ 1.3x A4 hinted | scriptc about 1.15x on nbody; C with JS FP rules about 1.22x [R15][R17]. Only nbody has supporting data (richards ≈ Maglev; raytracer 1.54x slower; deltablue none) [R10][R15] | K-A5pre, K-A5v-cov, K-A5v-bound, K-A5v, K-A5 |
| Not claimed | - | - | Peak on general, polymorphic, untyped JS; hello-world startup and RSS as AOT effects | Static Hermes untyped 10-13x slower; Dart poly 1.94x; .NET NativeAOT 2.1-5.3x [R11][R15][R17] | N10 |

- At its floor (0.085 of V8 full), FV1 still sits below Static Hermes untyped native (0.094) and Hermes JIT (0.117) [R15].
- A4 also reports optimized-AOT against ep9's own T2 (no gate). This tests "AOT for even faster performance" directly. It is D15 input at M8.

### 1.3 Evidence at a glance

| Finding | Number | Tags |
|---|---|---|
| Static Hermes typed native vs V8 full | 1.5-3.2x slower (geomean about 2.1x); nbody 1.42x faster than Maglev cap; gap = implementation debt | [R15][R17] |
| scriptc typed subset, nbody | About 1.15x faster than V8 peak; 1.25-1.28x incl. V8 warm-up; refuses eval | [R10][R17] |
| Naive untyped AOT | Static Hermes untyped 10-13x slower than V8, 1.3-2.0x its interpreter; Porffor 11-18x slower | [R15][R17] |
| Hopc (untyped AOT + inference) | Many JetStream tests 1-2x slower than V8 6.8 (2018); current V8 likely widens the gap. Far ahead of naive untyped AOT; ≥ 3x less memory | [R10][R11][R17] |
| Graal PGO native-image | AWFY time vs C2: JIT 0.92, native-image 1.02, PGO 0.89-0.91 (trained on the same benchmarks) | [R11][R17] |
| Leyden profile replay | First iteration -31%; steady state 20-40% slower (Havlak) | [R11] |
| Native code vs bytecode bytes | Sparkplug 6.9x, Maglev 11.6-19x, TurboFan 11.9-28.6x, Static Hermes typed 3.4-4.7x | [R14] |
| Warm-up share | tsc: 26-30% of a 1.6 s run. V8 kernels reach peak in 2-3 ms | [R15] |

Full answer, ladders, trust levels, unit format and stages: [plan/05-aot.md](plan/05-aot.md).

## 2. Answers to your questions

**Q1. Use: embeddable engine plus a Node/Deno-like runtime?** Yes ([plan/07-runtime.md](plan/07-runtime.md)).
- Embedders use an unstable `ep9` facade (N1). Units and snapshots also load through it.
- Runtime: tokio loop per isolate thread, ESM + CJS + `.ts`, `node:*`, web APIs, workers, inspector (M4), Node-API (M6).
- Gates: tsc and prettier output equals node in each shipped mode. node/test ≥ 80% (M4), ≥ 90% (M6). `ep9 -e 0` ≤ 1.0x node (est.).

**Q2. Spec: full ECMAScript incl. Intl and Temporal?** Yes ([plan/06-object-model-and-spec.md](plan/06-object-model-and-spec.md), [plan/10-quality.md](plan/10-quality.md)).
- The SPEC lane uses agents with test262 as the oracle. Intl uses ICU4X 2.3; Temporal uses temporal_rs 0.2.6 [R08].
- test262 per file: ≥ 97.5% at M3, ≥ 98.5% at v1, in every tier and shipped mode. V8 is at 97.69% [R07].
- eval, `new Function` and `toString` work in every mode, so the compiler and T0 are always linked (H5).

**Q3. Speed: can ep9 reach V8 class?** Close on the TS workload. Not on general JS (Octane-7) in v1 ([plan/04-execution.md](plan/04-execution.md)).
- v1 JIT mode targets about 39-55% of V8 full on Octane-7 (T2 = 10-14x QuickJS) and about V8 speed on the TS workload. Near-V8 Octane peak needs the optional T3 (M9, ≥ 20x QuickJS = about 78%).
- Contracts first (M0). Then T0 must reach V8-jitless parity: the M2 hard gate (Octane-7 ≥ 1.33x QuickJS).
- T1 at M5 (≥ 3.5x QuickJS). T2 at M7 (≥ 10x, aim 14x; TS workload ≤ 0.6 s). Optional T3 at M9 (≥ 20x).
- T2 is the tier that matters: Maglev/DFG reach about 54% of V8 full on Octane and match V8 on the TS workload (0.53 vs 0.55 s) [R06].
- AOT adds start, warm-up and jitless speed. It does not replace T2.

**Q4. tsrs as the JS/TS parser?** Not as the engine parser ([plan/01-front-end.md](plan/01-front-end.md)).
- tsrs catches 26.8% of test262 early errors (oxc: 100%). Its binder is always strict (868-889 false errors) [R01][R02][R04].
- Linked in process, it reserves a fixed 32 GiB. Its Unicode 15.1 pin makes oxc fail 16 test262 tests [R02][R09].
- Yes for `ep9 check` (tsrs CLI, out of process) and as the offline type oracle `ep9-typecheck`: 99.94% span join on 556,724 nodes [R13].
- The gap is by design: TypeScript reports most early errors in the checker, and tsrs is a faithful port [R01][R09].

**Q5. GC: "hybrid like V8"?** Hybrid like JSC, not V8 ([plan/03-gc.md](plan/03-gc.md)).
- Generations through sticky mark bits, concurrent retreating-wavefront marking (JSC Riptide, not SATB), non-moving first. Stages S0 -> S1 -> S2 -> S3 (opportunistic evacuation).
- Conservative scan of Rust, JIT and AOT frames; precise heap. Objects never move under live Rust pointers [R05][R09].
- V8's own young generation is still the copying Scavenger; MinorMS and sticky marks are experimental there [R05]. AOT changes no GC algorithm.

**Q6. Where is `unsafe`, and how is it contained?** ([plan/08-repo.md](plan/08-repo.md), [plan/10-quality.md](plan/10-quality.md))
- `unsafe` lives in reviewed crates: `ep9_platform`, `ep9_value`, `ep9_heap`, `ep9_vm`, `ep9_snapshot`, `ep9_masm`, `ep9_baseline`, `ep9_opt`, the `ep9_unit` loader, `ep9_napi`. `ep9_frontend` and `ep9_regexp` hold small contained parts.
- `ep9_builtins_*`, `ep9_intl` and `ep9_temporal` use `#![forbid(unsafe_code)]` and reach the heap only through ObjectOps.
- Checks: Miri, ASan, TSan, loom, GC-stress, barrier verifiers V1-V3 and a barrier model test, loader fuzzing, CODEOWNERS and adversarial review of every `unsafe` change.

**Q7. Out of scope?** A stable embedding API (N1), a WASM build target of ep9, and no_std (N2). The facade and the unit-loading API stay unstable.

**Q8. Dependencies in `third_party/`?** ([plan/08-repo.md](plan/08-repo.md))
- Each dependency has `third_party/<name>.commit`, `scripts/sync-<name>.sh` and optional patches. The root workspace excludes `third_party`.
- tsrs never enters the engine graph. Its CLI builds in tsrs's workspace; `ep9-typecheck` builds in `tools/ep9-typecheck`.
- oxc comes from crates.io with an exact `=0.155.0` pin and ≤ 3 local patches. CI guards check `cargo tree`.

## 3. Key diagram

```
 .js .mjs .cjs .ts .mts .cts                                   eval / new Function text (every mode, H5)
   |                                                              |
   v                                                              v
 ep9_frontend: oxc =0.155.0 parse + semantic | 3-layer depth guard | scope pass
               TS lowering in BytecodeGen (H3) | hint join on the pre-lowering AST <-- .ep9h (optional)
   |
   v
 CompiledUnit: bytecode ABI (H2) = code cache = unit bytecode; deterministic, hashed, stable function ids
   |
   +-- (a) RUNTIME, every mode ---------------------+-- (b) BUILD, ep9c -> ep9_aot (`aot`) -------------+
   |   finalize() -> T0 -> T1 -> T2 -> [T3]         |   module graph, hot set (D19)                     |
   |   FeedbackVector = FactSnapshot schema (H16)   |   PROVEN + PROFILED (.ep9prof)                    |
   |   -> RUNTIME FactSnapshot                      |   + HINTED (.ep9h <- ep9-typecheck <- tsrs)       |
   +-----------------------+------------------------+-----------------------+---------------------------+
                           v                                                v
     ONE optimizer: ep9_baseline (T1 | baseline-AOT)   ep9_opt (T2, [T3] | optimized-AOT)
     facts: proven ⊑ predicted{source}; RUNTIME > PROFILED > HINTED; guard policy per mode (H30, H32)
     no raw heap constants: RootRef IntrinsicRef ShapeRef CellRef UnitConst | FP: no FMA (H36)
                           |
                           v
     ep9_masm: one instruction stream, abstract relocation kinds (H34)
       +-- JIT finalize (`jit`): W^X code range       +-- AOT finalize (`aot`): zero text relocations
       v                                              v
     JIT code; tier-up = dispatch-slot swap         unit .o / .ep9u (H35): header | text RX | rodata |
                                                    RW link + IC cells | bytecode | source | hints

 ALWAYS LINKED (every build and mode): T0 + bytecode compiler + source | dispatch slots
   {interp, baseline, opt, aot-baseline, aot-opt} | pc->code registry | deoptimizer | fuses | unit loader
 HEAP/GC: cage below 2^47 + code range | non-moving first | CSS of Rust + JIT + AOT frames | precise heap
   and unit RW data | one store barrier (JSC Riptide form) in every tier and code kind | S0 -> S1 -> S2 -> S3
 RUNTIME LAYER: event loop | ESM/CJS/.ts | node:* | Node-API | `ep9 check` --spawn--> tsrs CLI
```

Full diagram and crate layers: [plan/02-architecture.md](plan/02-architecture.md).

## 4. Three deployment modes (A1 artifacts serve all three)

| | JIT mode (default) | AOT-jitless mode | Hybrid mode |
|---|---|---|---|
| Tiers | T0 -> T1 -> T2 (-> optional T3) | T0 + baseline-AOT (A3) + optimized-AOT (A4), built by `ep9c` | Units start; hot code re-tiers through instrumented T1 to T2 |
| Facts | PROVEN, RUNTIME (+ HINTED only if K-A4H keeps it) | PROVEN, PROFILED, HINTED (build time) | PROVEN, PROFILED, HINTED at build; RUNTIME after re-tier |
| Guard failure | DEOPT + exit counter | SLOWPATH or DEOPT within budget, then PIN | DEOPT, then instrumented re-tier |
| Peak owner | JIT (T2, later T3) | optimized-AOT (baseline-AOT if K-A4J fails) | JIT (T2) after re-tier |
| Ships | `ep9` JIT build + A1 artifacts | `ep9` jitless build + units (.o or .ep9u) | `ep9` JIT build + units |
| v1 status | Ships | Ships; baseline-AOT only if K-A3 passes (else M5c or T0); optimized-AOT only if K-A4J passes | Only if K-A4P passes |

## 5. Goals at a glance

| ID | Goal (measures in [plan/02-architecture.md](plan/02-architecture.md) §1) |
|---|---|
| G1 | Full ECMAScript in every tier and shipped mode: test262 ≥ 97.5% (M3), ≥ 98.5% (v1) |
| G2-G4 | T0 ≥ 1.33x QuickJS (V8 jitless); T1 ≥ 3.5x, TS ≤ 1.1 s; T2 ≥ 10x (aim 14x), TS ≤ 0.6 s |
| G5-G6 | Engine + Node/Deno-like runtime; hostile input gives SyntaxError or RangeError, never an abort |
| G7 | NaN-box memory: RSS ≤ 1.2x node --jitless (est.) and ≤ 1.8x compressed d8 --jitless |
| G8 | No contract rewrite after M1 |
| G9-G10 | First-class jitless build and AOT-jitless mode; 4 tier-1 targets, iOS only if D16 = yes |
| G11 | Dual-mode contracts from M0: SP-Unit at M0, SP-Facts at M1, 0 H-row rewrites after M1 |
| G12 | Three modes in v1: test262 equal per mode x code kind; hybrid only if K-A4P passes |
| G13 | typescript.js + 1 call loads from a unit ≥ 2x faster than ep9's cold source path (est.) |
| G14 | FV1-FV3 each meet a numeric gate with a named comparator, or ep9 does not claim them |
| G15 | Size, startup and RSS reported per mode from A1; payload and total shipped size kept apart. The M8 report adds latency and AOT vs own JIT |
| N10-N13 | No general "AOT beats V8" claim; no unguarded TS types; no AOT of every function by default; no lean runtime without compiler and T0 |

## 6. Top hard decisions

| ID | Decision | Choice | Fix by |
|---|---|---|---|
| H1 | Front-end boundary | AST stays in `ep9_frontend`; GC-free CompiledUnit with ABI header | M0 wk3 |
| H2 | Bytecode is a serialized ABI | Deterministic, hashed, versioned; stable function ids; offset = deopt id | M0 wk3; CI at M0 |
| H3 | TS runtime semantics | Lowered in BytecodeGen; TS flags in header, `.ep9h` and cache key | Fields M0 wk3; built M4 |
| H5 | Compiler, T0, source | Always linked in every build and mode | M0 wk3 |
| H6 | Value encoding | JSC NaN-box in `Value(NonZeroU64)`; empty/hole = 0x22; no 0 in a heap slot | M0 wk2 |
| H8 | Header and ShapeId | Stable ShapeIds only for snapshot root shapes; AOT guards compare RW cells | M0 wk2 |
| H10/H11 | Rooting and barrier | CSS of native frames; one store barrier (JSC Riptide form) in every code kind | M0 wk2; barrier API M0 |
| H14 | Frames and dispatch | One frame for T0-T3 and both AOT kinds; dispatch-slot kinds; pc->code registry | M0 wk3; Spikes B, C |
| H16 | ICs and feedback | IC state only in RW data; feedback schema = FactSnapshot schema | Frozen M1 exit |
| H29 | Deopt contract | Position-independent metadata; lazy deopt by return-address swap only | Frozen M1 exit |
| H30 | IR fact model | proven ⊑ predicted{source}; no raw heap constants | Frozen M1 exit |
| H32 | Guard failure per mode | JIT DEOPT; hybrid re-tier; jitless SLOWPATH, then PIN; never THROW on hints | Frozen M1 exit |
| H35 | AOT unit format | Zero text relocations; RW link/IC cells; versioned header; on a mismatch, the field picks bytecode, source or a clean load error | Spec v0 M0 exit; frozen at A3 gate |
| H36 | FP semantics | No FMA; exact IEEE-754 and ToInt32; one Rust Math (`ep9_math`, correctly rounded) | M0 wk2 |
| H37 | Hint side table | `ep9_hints` binary schema; key (start, end, kind) + text hash | v0 by M4 exit; frozen at A4 start |

All 37 rows, with evidence: [plan/02-architecture.md](plan/02-architecture.md) §6.

## 7. Milestone map

```
        M0         M1            M2           M4           M5          M7            M8          optional
CORE   [M0]------>[M1]-------->[M2]==gate==>(M4 items)-->[M5]------->[M7]--------->[M8 v1]----> [M9]
3.5    contracts  perf slice   T0 parity    snapshot,    T1 + S2b    T2 + facts    hardening,
FTE    +A0 (1)    +Spike C     (hard gate)  L1, debug    + RX2       +SP-TypedBound mode gates
                  +A0 (2)
SPEC             [M1a]---------------------->[M3 compliance] (closes after M2)
RT               [M1]-->[M4 runtime alpha + A1]------------>[M6 runtime beta]---------> [M8 share]
AOT    A0(1) --> A0(2) --> A1h + A2 ------> A1 ---------> A3 --------> A4 ---------> gates -----> [A5=M10]
1.0                       (after M1 exit)  (in M4)       (with M5)    (with M7)      at M8 exit
Rules: no JIT tier code runs JS before M2. Unit machinery (A2) may start after M1 exit behind `aot`.
       The AOT lane never gates M2, M5 or M7. It gates M0/M1 exits only via time-boxed A0 spikes.
       K-A3 pre-check at M5 exit in every case; if it trips, M5c (3-6 PM est.) runs in the M7 window.
```

| M | Name | AOT stage inside | Key exit gate | PM (est.) | Lane |
|---|---|---|---|---|---|
| M0 | Contracts, spikes, skeleton | A0 part 1 | Spikes green incl. SP-Unit on 4 targets; identical bytecode hashes | 12.5-19 | all |
| M1 | Perf vertical slice (M1a checkpoint) | A0 part 2 | Octane-7 ≥ 1.0x QuickJS; Spike C + SP-Facts green; H29-H34 frozen | 12.5-22 | CORE |
| M2 | Interpreter parity (hard gate) | - | Octane-7 ≥ 1.33x QuickJS; v8-v7 ≥ 1.0x node --jitless; TS ≤ 2.8 s | 14-28 | CORE |
| M3 | Compliance complete | - | test262 ≥ 97.5% per file | 22-42 | SPEC |
| M4 | Runtime alpha | A1; A1h due | Unit load ≥ 2x (G13); `ep9 -e 0` ≤ 1.0x node; join ≥ 99.5% (all est.) | 18-36 (+AOT 2-4) | RT |
| M5 | Baseline JIT + GC S2b | A2 due at start; A3 built | Octane-7 ≥ 3.5x QuickJS; TS ≤ 1.1 s; A3 gate if D16 = yes; K-A3 pre-check recorded | 26-52 | CORE |
| M6 | Runtime beta | - | node/test ≥ 90%; napi-rs suites pass | 14-28 | RT |
| M7 | Speculative mid tier + fact inputs | A4 built; SP-TypedBound, SP-RegionBound | Octane-7 ≥ 10x QuickJS; TS ≤ 0.6 s; test262 equal from PROFILED | 120-200 (+A4 16-32) | CORE |
| M8 | v1 hardening and release | A3, A4 gates; AOT ship review | test262 ≥ 98.5% and equal per shipped mode x code kind; G15 report | 18-40 | all |
| M9 | Top tier (optional) | serves optimized-AOT | Octane-7 ≥ 20x QuickJS | 120-360+ | CORE |
| M10 | Sound typed code (optional) | = A5, by the D17 rule | FV3; K-A5v or K-A5 | (c) 12-24; (d) 16-33 | AOT |

v1 (M8) ships JIT mode, A1 artifacts and AOT-jitless mode. Its jitless top tier is baseline-AOT if K-A3 passes, else M5c if ready, else T0. Optimized-AOT, hybrid mode and HINTED facts ship only if K-A4J, K-A4P and K-A4H pass. The orchestrator applies each kill rule at the M8 AOT ship review and logs it. You can override any result. Detail: [plan/09-milestones.md](plan/09-milestones.md).

## 8. Gates and kill criteria (all thresholds est.)

| ID | When | Trips if | Then |
|---|---|---|---|
| K-A3 | Pre-check at M5 exit (every case); final at the A3 gate | baseline-AOT < 1.3x T0 on JITLESS-SET | Stop baseline-AOT emit; M5c generated interpreter (3-6 PM est., AOT lane) becomes the jitless top tier |
| K-A4J | M8 exit | optimized-AOT < 1.5x baseline-AOT on JITLESS-SET | Jitless ships baseline-AOT only |
| K-A4P | M8 exit | Hybrid misses FV2 time-to-peak or steady state, or test262 differs | No hybrid mode in v1 |
| K-A4H | M8 exit (early off-ramp at A1h: class receivers and slot found < 5% on ≥ 2 of 3 real-app groups) | < 3% HINT-SET gain in every mode and < 10% better time-to-peak; or any HINT-SET workload > 1% slower; or upkeep > 1 PM/yr | Remove HINTED consumers; keep `ep9 check` |
| K-A5v-cov | A1h (SP-RegionCoverage) | hot share in finder-v1 regions < 15% on ≥ 2 of 3 real-app groups | No real-app A5v; kernels only by the D17 rule |
| K-A5pre | M7 (SP-TypedBound) | T2 without guards < 1.15x normal T2 on TYPED-KERNELS | No (c) (D17 rule) |
| K-A5v-bound | M7 (SP-RegionBound) | region code < 1.1x A4 code inside regions on TYPED-KERNELS | No A5v |
| K-A5v | After the A5v prototype | Real-app gain < 3% AND FV3 missed | Stop A5v; one failed part drops only its claim |
| K-A5 | After the A5 (c) prototype | FV3 missed | Stop A5 (c) |

- **K-A3 risk is real.** Naive untyped AOT gives 1.3-2.0x its interpreter, so the low end sits on the K-A3 line [R15][R17].
- **M2 hard gate.** No JIT tier code before T0 parity; one slow kernel becomes parity debt, cleared by M5 exit.
- **P-gates P1-P5.** An own parser starts only if oxc fails one: patch cap, bump cost, compile speed, lone surrogates, parse share of cold start.

## 9. Effort (PM as a size of work; D3)

| Lane | Work | PM (est.) | FTE | Months |
|---|---|---|---|---|
| CORE | M1 -> M2 -> M4 items -> M5 -> M7 -> M8 share | 188-339 | 3.5 | 54-97 (critical path) |
| SPEC | M3 | 22-42 | 0.5 | in parallel |
| RT | M4 runtime -> M6 | 28-56 | 1 | in parallel |
| AOT | A1h, A2, A1, A3, A4 (A0 sits in M0/M1); + M5c 3-6 only if K-A3 trips, partly offset by stopped A3 work (est.) | 28-57 | 1 | inside CORE's window |

- Total M0-M8: about 285-524 PM (est.), a size of work. The FTE and Months columns are human-team figures. Under D3 they do not apply.
- If A4 trails M7, v1 waits for the A4 gates (D15 timing t1). A4 parts that fail K-A4J, K-A4P or K-A4H leave v1 by their kill rule.

## 10. Decisions (all decided 2026-10-20)

You said: "Don't ask me questions. Deepresearch and descide yourself". So the agent team decided all 21 items. Each is final until its re-check point. There, gate data applies a written rule. You can override any entry. Full table: [plan/11-decisions.md](plan/11-decisions.md). Log: [decisions-log.md](decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction).

| ID | Question | Decision | Re-check |
|---|---|---|---|
| D1 | Parser path | E: oxc pinned, ≤ 3 patches, own parser only on a P-gate | P-gates |
| D2 | v1 platforms | macOS arm64, Linux x64/arm64, Windows x64; iOS only if D16 = yes. A Windows gate counts only after a Windows run | first Windows run |
| D3 | Team | AI agents led by an orchestrator; you review at will. Required reviews follow ADR-P1 (reviewer agent + Codex adversarial review). PM = size; no calendar | M0 exit |
| D4 | Node depth | ESM, CJS, V8 stack API (M4); Node-API (M6); legacy decorators (no hints); rule-based node/test exclusions | M4 start |
| D5 | WebAssembly | wasmtime JS API after v1 | before M4 |
| D6 | Sandbox | None in v1; units are trusted code | M1a (your request only) |
| D7 | Tail calls | Accept the gap | - |
| D8 | Toolchain | Stable 1.99.1 | - |
| D9 | C++ | Pure Rust | M5 exit (RX1 debt rule) |
| D10 | Intl data | Full baked data | - |
| D11 | JIT-free mode | Yes; gate data picks only its top tier | K-A3, A3 gate, M8 exit |
| D12 | Upstream | No contact; no public posts; local patches + drafts | P1 |
| D13 | Baseline tier | Build T1 | M2 exit |
| D14 | Worker heap | Isolate per thread | - |
| D15 | AOT claim; v1 timing | (a); (b) only as gated A5; (c) rejected. Timing (t1): v1 waits for the A4 gates; A4 parts that fail K-A4J, K-A4P or K-A4H leave v1 by their rule; (t2) only on your request | M8 exit |
| D16 | iOS near-term | No; v1.x only by rule | M5 exit, A3 gate |
| D17 | Sound typed code | M7-exit rule: (d) automatic VERIFIED regions > (c) opt-in subset > (a) none; (b) rejected | A1h, M7 exit, M8 exit |
| D18 | Hint investment | (b), with the A1h off-ramp | A1h, M8 exit |
| D19 | Compile scope | Hybrid: hot set. Jitless: baseline-AOT all + optimized hot set; A3 size switch | A3 SIZE-REPORT |
| D20 | Class fields | Follow tsconfig, else [[Define]] | - |
| D21 | Frame home | (a) Hermes model; Spike B places JIT frames | Spike B |

Gate data does not come back to you as a question. At each re-check point the orchestrator applies the written rule and logs the result.

## 11. What changed from plan v1

- Dual mode is the top goal: one engine, three modes, one optimizer. A0 contracts sit in M0/M1.
- tsrs gains a third role: the offline type oracle `ep9-typecheck` (option H).
- TS runtime lowering moves from oxc_transformer into BytecodeGen (H3).
- v1's 27 contract rows become H1-H37: 10 new, 14 extended, 13 unchanged (H23 became extended on 2026-10-20).
- New crates: `ep9_facts`, `ep9_hints`, `ep9_unit`, `ep9_aot`, `ep9c`. Renamed: `ep9_baseline`, `ep9_opt`. The deoptimizer runtime moves to `ep9_vm`.
- New AOT lane and optional M10. SP-Unit joins Spike B; SP-Facts joins Spike C.
- New gates FV1-FV3 and kill criteria K-A3 to K-A5. Default team grows from 5 to 6 seniors.
- New decisions D15-D21. On 2026-10-20 the agent team decided D1-D21 under your instruction; re-check points apply written rules ([decisions-log §5](decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)). Renamed IDs: lanes CORE/SPEC/RT (v1 A/B/C), RegExp RX0-RX2, risks RK1-RK44.
- New fallback: a K-A3 pre-check at M5 exit and M5c as the jitless top tier if baseline-AOT fails.

## 12. Document map

| File | Purpose |
|---|---|
| [plan/01-front-end.md](plan/01-front-end.md) | tsrs verdict, oxc boundary, depth guard, P-gates, TS lowering, hint join |
| [plan/02-architecture.md](plan/02-architecture.md) | Goals, big diagram, fact sources, modes, H1-H37, A0 cost |
| [plan/03-gc.md](plan/03-gc.md) | GC answer, roots, barrier, stages S0-S4, snapshot interaction |
| [plan/04-execution.md](plan/04-execution.md) | Tiers, RegExp, relocation kinds, fact model, guards, fuses, deopt, FP |
| [plan/05-aot.md](plan/05-aot.md) | The core idea: FV cases, trust levels, oracle, unit format, A0-A5, kills |
| [plan/06-object-model-and-spec.md](plan/06-object-model-and-spec.md) | Values, ShapeId rule, strings, numbers, build vs reuse |
| [plan/07-runtime.md](plan/07-runtime.md) | Runtime components, A1, commands per mode, acceptance, iOS |
| [plan/08-repo.md](plan/08-repo.md) | Workspace tree, crates, binaries, tsrs and oxc consumption, guards |
| [plan/09-milestones.md](plan/09-milestones.md) | Lanes, M0-M10, A-stages, spikes, staffing, M0 week by week |
| [plan/10-quality.md](plan/10-quality.md) | test262 per mode, suites, benchmark sets, fuzzing, risks RK1-RK44 |
| [plan/11-decisions.md](plan/11-decisions.md) | D1-D21 with options, decisions, re-check rules and evidence |
| [plan/12-governance.md](plan/12-governance.md) | Licence, repository, CI cost, review and ownership, upstream contact, line endings |
| [plan/13-execution-modes.md](plan/13-execution-modes.md) | Execution modes: build features, process permit, ExecPolicy presets, code sets and `select()`, facts across runs, append-only contract parts, future room F1-F24 |

| Tag | Report | Tag | Report |
|---|---|---|---|
| [R01] | [research/01-tsrs-architecture.md](research/01-tsrs-architecture.md) | [R10] | [research/10-js-aot-precedents.md](research/10-js-aot-precedents.md) |
| [R02] | [research/02-tsrs-conformance.md](research/02-tsrs-conformance.md) | [R11] | [research/11-dual-mode-vms.md](research/11-dual-mode-vms.md) |
| [R03] | [research/03-parser-perf-and-features.md](research/03-parser-perf-and-features.md) | [R12] | [research/12-ts-type-soundness.md](research/12-ts-type-soundness.md) |
| [R04] | [research/04-parser-alternatives.md](research/04-parser-alternatives.md) | [R13] | [research/13-tsrs-type-oracle.md](research/13-tsrs-type-oracle.md) |
| [R05] | [research/05-gc-design.md](research/05-gc-design.md) | [R14] | [research/14-dual-mode-architecture.md](research/14-dual-mode-architecture.md) |
| [R06] | [research/06-execution-tiers.md](research/06-execution-tiers.md) | [R15] | [research/15-aot-perf-reality.md](research/15-aot-perf-reality.md) |
| [R07] | [research/07-rust-engine-landscape.md](research/07-rust-engine-landscape.md) | [R16] | [research/16-aot-synthesis.md](research/16-aot-synthesis.md) |
| [R08] | [research/08-spec-subsystems.md](research/08-spec-subsystems.md) | [R17] | [research/17-aot-refutation.md](research/17-aot-refutation.md) |
| [R09] | [research/09-critic.md](research/09-critic.md) | [R18] | [research/18-sound-regions.md](research/18-sound-regions.md) |
| | | | [research/README.md](research/README.md) (index) |

[R17] corrects [R16]. [R09] arbitrates among [R01]-[R08]. [P1] = plan v1 local probe (session-local), re-checked by the named M0 spike.
