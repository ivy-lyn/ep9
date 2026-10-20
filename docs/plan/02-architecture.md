# ep9 plan: dual-mode architecture

**Evidence tags:** [Rnn] = `../research/nn-*.md`. [P1] = plan v1 local probe. Full table at the end.

- ep9 is one engine with three deployment modes: JIT, AOT-jitless and hybrid. All three share one optimizer [R11], [R14].
- Modes differ only in fact sources (PROVEN, RUNTIME, PROFILED, HINTED) and in guard policy. They share the bytecode ABI, frames, ICs, deopt metadata and GC.
- The JIT owns peak on general JS. AOT owns startup, jitless speed, predictable latency and no JIT compile CPU (D15 = a) [R15], [R17].
- 37 hard decisions (H1-H37) replace v1's 27 rows: 10 new, 14 extended, 13 unchanged (H23 became extended on 2026-10-20). Each becomes an ADR.
- The dual-mode contracts are fixed in M0/M1 for about 1-2 PM (est.). Everything else is staged A1-A5.

IDs used here: G1-G15, N1-N13; H1-H37; C1-C14; D2, D4, D6, D14-D21; A0-A5; T0-T3; FV1-FV3; K-A3, K-A4J, K-A4P, K-A4H; M5c; SP-Unit, SP-Facts; VERIFIED; S0-S4; RX1, RX2; SIZE-REPORT, STARTUP-SET, SHORT-RUN.

---

## 1. Goals and non-goals

| ID | Type | Statement | Measure or reason |
|---|---|---|---|
| G1 | Goal | Full ECMAScript (ES2026, ES2027 so far, Annex B, Intl, Temporal) in every tier and shipped mode | test262 full suite (53,616 files, 103,175 runs). A file passes when every strict/sloppy run it needs passes; staging and N3 features count as failures. ≥ 97.5% at M3; ≥ 98.5% at v1 (M8). Scale (per file, test262.fyi): V8 97.69%, QuickJS 98.48%, JSC 98.80% [R07] |
| G2 | Goal | Interpreter as fast as V8 jitless | Octane-7 ≥ 1.33x QuickJS. v8-v7 7-kernel geomean (no RegExp) ≥ 1.0x node --jitless. RegExp kernel ≥ 0.8x node --jitless by M5 exit [R06], [R09] |
| G3 | Goal | Baseline JIT of JSC-Baseline class | Octane-7 ≥ 3.5x QuickJS. TS workload ≤ 1.1 s [R06] |
| G4 | Goal | Speculative mid tier of Maglev/DFG class | Octane-7 ≥ 10x QuickJS (aim 14x). TS workload ≤ 0.6 s. JetStream 3 M7 list ≥ 0.75 × the node --max-opt=2 / node full ratio (est.) [R06] |
| G5 | Goal | Embeddable engine plus a Node/Deno-like runtime | ESM + CJS, event loop, inspector, snapshots, Node-API. node/test, WPT and real-app gates ([07-runtime.md](./07-runtime.md)) |
| G6 | Goal | Production behavior on hostile input | Deep nesting of every form gives SyntaxError or RangeError, never an abort. Each fuzz budget ends with 0 unique crashes |
| G7 | Goal | NaN-boxing memory cost stays bounded | Peak RSS on v8-v7 from M2: ≤ 1.2x node 24 --jitless (est.) and ≤ 1.8x a pointer-compressed d8 --jitless. A failed cap at M2 moves S3 into M5 [R06] |
| G8 | Goal | No contract rewrite after M1 | The ADR log shows 0 rewrites of H rows after M1 exit |
| G9 | Goal | A first-class jitless build and AOT-jitless mode | Compilers are Cargo features (`jit`, `aot`). test262 pass set equal with and without the JIT, and in AOT-jitless mode |
| G10 | Goal (D2) | Tier-1: macOS arm64, Linux x64, Linux arm64, Windows x64. iOS arm64 (AOT-jitless) only if D16 = yes | CI tests all 4 from week 1. Speed gates hard on macOS arm64 and Linux x64; Windows x64 within 10% of Linux x64 (est.). `ep9 check` and ep9-typecheck unix-only until SP-tsrs-win |
| G11 | Goal | Dual-mode contracts fixed from M0 | SP-Unit green at M0 exit. SP-Facts green at M1 exit. 0 H-row rewrites after M1 |
| G12 | Goal | Three modes in v1 | test262 pass set equal per shipped mode × code kind. Hybrid ships only if K-A4P passes |
| G13 | Goal | AOT startup | typescript.js + 1 call loads from a unit ≥ 2x faster than ep9's cold source path (est.) [R10], [R17] |
| G14 | Goal | Faster-than-V8 cases are gated | FV1-FV3 each meet a numeric gate with a named comparator, or ep9 does not claim them ([05-aot.md](./05-aot.md)) |
| G15 | Goal | Size, startup and RSS reported per mode | SIZE-REPORT and STARTUP-SET per mode from A1. Per-app payload and total shipped size are kept apart. No size target before the A3 review. The M8 report adds the SHORT-RUN latency metric and the AOT-vs-own-JIT speed report ([05-aot.md](./05-aot.md) §3) |
| N1 | Non-goal (user) | A stable embedding API | Only an unstable `ep9` facade |
| N2 | Non-goal (user) | A WASM build target of ep9 itself; no_std | none |
| N3 | Non-goal | TC39 decorators, ShadowRealm, immutable ArrayBuffer (all Stage 2.7) | Check again at each test262 bump [R08]. TS legacy decorators are a transform (D4) |
| N4 | Non-goal (v1) | A top tier of TurboFan/FTL class | Optional M9 |
| N5 | Non-goal (refined) | LLVM in the JIT or in AOT. Cranelift for T1/T2 | Cranelift only for the optional T3, shared by JIT and AOT emit. FTL left LLVM; Cranelift costs 11-21 µs per bytecode [R06] |
| N6 | Non-goal (v1) | V8-style pointer compression | G7 caps the cost instead |
| N7 | Non-goal | Spec deviations for speed (Hermes style) | Deopt only the affected function [R07] |
| N8 | Non-goal | tsrs in the engine parse path or in any engine crate | [01-front-end.md](./01-front-end.md) |
| N9 | Non-goal | Speedometer (it needs a DOM). Concurrent compaction, ever | JetStream 3 instead [R07]; [R05] |
| N10 | Non-goal | A general "AOT beats V8" claim | No measured JS AOT system beats V8 on general code [R15], [R17] |
| N11 | Non-goal | Trusting TS types without guards in conformance mode | TS is unsound by design; 24 known hole classes pass strict checks and lie [R12], [R13] |
| N12 | Non-goal | AOT of every function by default | Baseline and mid-tier native code is 6-19x the bytecode bytes; TurboFan with inlining reaches 28.6x [R14]. Defaults (D19): PGO hot set for hybrid and optimized-AOT. Jitless baseline-AOT covers reachable functions, bounded by the A3 gate (unit text ≤ 8x bytecode). This departs from [R14] for jitless cold-code speed [R16] |
| N13 | Non-goal | A lean runtime without compiler or interpreter in full-ECMAScript mode | eval, new Function and toString must work in every mode (H5) [R14] |

## 2. One engine, three modes

Path (a) runs in every mode. Path (b) runs at build time inside `ep9c`. Both paths feed the same compilers, the same backend and the same runtime.

```
 SOURCE .js .mjs .cjs .ts .mts .cts (UTF-8 files)         eval / new Function text (every mode, H5)
   |                                                         | lone-surrogate shim
   v                                                         v
+== FRONT END  ep9_frontend (only oxc importer; sized-stack worker thread) ==============================+
| oxc_parser =0.155.0 + oxc_semantic + regex (100% early errors) | 3-layer depth guard | scope pass      |
| hint join on the pre-lowering AST <-- .ep9h (optional, H37) | TS lowering in BytecodeGen (H3)          |
| BytecodeGen: deterministic, hashed, stable function ids (H2) -> CompiledUnit (GC-free)                 |
+==================================================+=====================================================+
                                                   | CompiledUnit: bytecode ABI, code cache, unit bytecode
                 +---------------------------------+----------------------------------+
                 | (a) RUNTIME PATH, every mode                                       | (b) BUILD PATH
                 v                                                                    v
 finalize() on the isolate thread                                 ep9c -> ep9_aot (`aot`): module graph,
 T0 interp -> T1 baseline -> T2 mid tier -> [T3]                  hot set (D19), .ep9prof (PROFILED),
 FeedbackVector = FactSnapshot schema (H16)                       .ep9h (HINTED), PROVEN facts
                 | RUNTIME FactSnapshot                                               | build FactSnapshot
                 +---------------------------------+----------------------------------+
                                                   v
+== SHARED COMPILERS  ep9_baseline (T1 + baseline-AOT)   ep9_opt (T2, [T3] + optimized-AOT) =============+
| ep9_opt graph builder: bytecode + FactSnapshot (H30) | facts: proven ⊑ predicted{source, confidence}   |
| Guard(kind, fact) -> guard policy per mode (H32)     | Assume(fuse) -> GuardFuse | InvalidatingFuse    |
| no raw heap constants: RootRef IntrinsicRef ShapeRef CellRef UnitConst | FP: no FMA, exact ToInt32     |
+==================================================+=====================================================+
                                                   v
+== ep9_masm: one instruction stream, abstract relocation kinds (H34) ===================================+
| JIT finalize (`jit`): W^X code range, HeapConst allowed | AOT finalize (`aot`): zero text relocations  |
+============================+===========================+================================+==============+
                             v                                                            v
      JIT code in the code range; tier-up = dispatch-slot swap     UNIT .o or .ep9u (H35): [header]
                                                                   [text RX][rodata: deopt + maps]
                                                                   [RW: link table, IC cells, counters]
                                                                   [bytecode][source][hints, optional]
                                                                   [unwind/debug]

 FACT SOURCES   precedence RUNTIME > PROFILED > HINTED                 guard on failure (H32)
   PROVEN    ep9 analysis (const, module bindings, int32 ops) .......  none
   RUNTIME   IC feedback from T0, T1, baseline-AOT ................... DEOPT + exit counter (JIT, hybrid)
   PROFILED  .ep9prof from a training run ............................ per mode: DEOPT | SLOWPATH | PIN
   HINTED    .ep9h TS types (unsound by design) ...................... per mode; never THROW

 [separate workspace]  tsconfig -> ep9-typecheck (links tsrs by path) -> .ep9h side table (ep9_hints)

+== RUNTIME  ep9_vm (always linked, every mode, H5) =====================================================+
| T0 + bytecode compiler + source text | dispatch slots {interp, baseline, opt, aot-baseline, aot-opt}   |
| pc->code registry (JIT code + unit ranges) | deoptimizer | fuse registry | unit loader (ep9_unit)      |
+========================================================================================================+
+== HEAP / GC  ep9_heap =================================================================================+
| cage below 2^47 + code range (H7) | 256 KiB pages + side bitmaps | one store barrier (H11)             |
| roots: CONSERVATIVE = Rust + JIT + AOT frames | PRECISE = VM stack, heap, Global<T>, unit RW data      |
| S0 STW mark-region -> S1 sticky marks -> S2 concurrent marking (retreating wavefront) -> S3 evacuation |
+========================================================================================================+
 ep9_platform: VA reserve, MAP_JIT + W^X (`jit`), unit mapping + unwind registration, spill trampolines
+== RUNTIME LAYER  ep9_runtime / ep9_node / ep9_inspector / ep9_napi / ep9_cli ==========================+
| event loop | ESM + CJS + .ts | node:* | web APIs | workers | startup snapshot + code cache (A1)        |
| `ep9 check` --spawn--> tsrs CLI (own workspace; unix until SP-tsrs-win) | Node-API                     |
+========================================================================================================+
```

## 3. Fact sources

| Source | Producer | When | Trust | Guard on failure | Precedence |
|---|---|---|---|---|---|
| PROVEN | ep9's own analysis: never-reassigned module bindings, `const`, scopes without direct eval/`with`, bitwise ops → int32, range analysis, spec invariants | JIT compile and `ep9c` build | proven | none | always holds |
| RUNTIME | FeedbackVector / IC data written by T0, T1 and baseline-AOT ICs (H16, H33) | run time | predicted | DEOPT + exit counter (JIT, hybrid). No consumer in jitless | 1 (highest) |
| PROFILED | `.ep9prof` from `ep9 run --profile-out` (training run) | build time | predicted | per mode (H32) | 2 |
| HINTED | `.ep9h` from ep9-typecheck (tsrs), joined in `ep9_frontend` | build time | predicted, HINT trust | per mode (H32); never THROW | 3 (lowest) |

- Facts live in one lattice: proven ⊑ predicted{source, confidence} (H30). On a conflict, runtime feedback always overrides a training profile and a hint (H33). Fused hint facts that feedback cannot give stay (H30 merge rule, [05-aot.md](./05-aot.md) §11).
- Sound facts exist only in the gated stage A5, by the D17 rule at M7 exit. Option (d): automatic `VERIFIED{region}` facts, guarded by entry checks and fuses; a failed check takes the slow path or deopts, never THROW. Option (c): opt-in SOUND modules, where checked facts need no guard and boundaries THROW (H32).
- The full guard table per fact and mode is in [04-execution.md](./04-execution.md); budgets are in [05-aot.md](./05-aot.md).

## 4. Deployment modes

| | JIT mode (default) | AOT-jitless mode | Hybrid mode |
|---|---|---|---|
| Tiers and code kinds | T0 → T1 → T2 (→ optional T3) | T0 + baseline-AOT (A3) + optimized-AOT (A4), built by `ep9c` | Units are the start tier; hot functions re-tier through instrumented T1 to T2 |
| Fact sources | PROVEN, RUNTIME (+ HINTED from a unit only if K-A4H keeps it) | PROVEN, PROFILED, HINTED (build time only) | PROVEN, PROFILED, HINTED at build; RUNTIME after re-tier |
| Guard failure (full table: [04](./04-execution.md)) | DEOPT + exit counter | SLOWPATH or DEOPT within budget, then PIN to baseline-AOT | DEOPT, then instrumented re-tier |
| Peak owner | JIT (T2, later T3) | optimized-AOT (baseline-AOT if K-A4J kills it) | JIT (T2) after re-tier |
| What ships | `ep9` JIT build + bytecode, code cache, snapshot | `ep9` jitless build + units (.o or .ep9u) | `ep9` JIT build + units |
| Use | Servers, CLI, development | iOS (if D16), W^X hosts, embedders that forbid JIT | Servers and CLI that want fast start and full peak |
| Speed claims | G2-G4 unchanged from v1 | FV1 (vs node --jitless) | FV2 (short runs vs node with a warm compile cache) |
| v1 status | Ships | Ships. baseline-AOT only if K-A3 passes (else M5c if ready, else T0); optimized-AOT only if K-A4J passes | Ships only if K-A4P passes |

A1 artifacts (code cache, startup snapshot, bytecode units) serve all three modes. FV3 (sound typed numeric class code, A5 = M10, by the D17 rule) is a gated research stage ([05-aot.md](./05-aot.md)).

## 5. Mode × code-kind matrix

| Code kind | JIT mode | AOT-jitless mode | Hybrid mode |
|---|---|---|---|
| T0 interpreter | Start tier; deopt target | Cold code, eval, fallback, deopt target | Eval, cold code, fallback, deopt target |
| T1 baseline JIT | Yes (M5) | No (no runtime codegen) | Yes: instrumented re-tier (H33) |
| T2 mid tier | Yes (M7) | No | Yes: re-tier target |
| T3 top tier | Optional (M9) | No | Optional (M9) |
| baseline-AOT | No | Yes: all reachable functions (D19), A3; only if K-A3 passes | Optional: A3 units also load; their ICs write RUNTIME feedback (H33) |
| optimized-AOT | No | Hot set, only if K-A4J passes (A4) | Yes: PGO hot set as start tier (A4) |
| A1 artifacts | Yes (M4) | Yes | Yes |
| test262 equal (G12) | Every tier | Every code kind | Every code kind, if shipped |

## 6. Hard-to-change decisions H1-H37

Each row becomes an ADR in `docs/adr/`. After it is frozen, a row changes only through an ADR amendment backed by a benchmark. Status: U = unchanged, E = extended, N = new.

| ID | Decision | Choice | Why | Evidence | Fix by | Origin | Status |
|---|---|---|---|---|---|---|---|
| H1 | Front-end boundary | AST never leaves `ep9_frontend`. CompiledUnit is GC-free and serializable; it is the code cache and the unit bytecode format. `finalize()` on the isolate thread. Header adds bytecode ABI version, decoded-source hash, TS flags, stable function ids, optional hint section | Swappable parser. Parse on big-stack threads. Cache and units reuse one format | [R04] Nova pin; [R01] region 5.8-6.8x source; [R09] verdict; [R16] | M0 wk3 | v1#1 | E |
| H2 | Bytecode is a serialized ABI | v1 format: register + accumulator, 1-byte opcodes, wide prefixes, feedback-slot operands, relative jumps, side tables, constant-pool indices only, `TailCall` reserved. Plus deterministic generation (same decoded source + flags + ABI version → same bytes), content hash, versioned ABI, serialized liveness. Bytecode offset = only deopt/OSR id. Stable function id = preorder index. Units ship bytecode of every compiled and inlined function | About 46% fewer dispatches than a stack VM. Bytecode is the deopt target of every code kind. Graal had to preserve AOT bytecodes before it could inline them | [R06] HTC2; [R09] snapshots, PTC; [R14] bytecode ABI, Ristretto; [R16] | M0 wk3; determinism CI M0; Spike C checks before M1 exit | v1#9 + R16-D1 | E |
| H3 | TS runtime semantics in BytecodeGen; TS flags in module identity | BytecodeGen lowers enums, namespaces, parameter properties, declare fields, `[[Set]]`/`[[Define]]` fields from the TS AST. oxc_transformer only for legacy decorators (no hints in v1). `useDefineForClassFields`, `experimentalDecorators`, `emitDecoratorMetadata` per module in CompiledUnit header, `.ep9h` header and cache key. Default `[[Define]]` without tsconfig (D20) | oxc_transformer gives lowered fields and enums dummy spans, so hint keys are lost. Same source gives different shapes; 4 of 6 codebases use `[[Set]]` | [R17]; [R12]; [R16] | Header fields M0 wk3; ADR M0 exit; built M4 | R16-D14 + R17 | N |
| H4 | Lazy-compile contract | SFI holds optional flushable bytecode, source range, ScopeInfo and context flags. Units carry eager bytecode | The lazy policy (L0-L2) changes without touching the object model | [R09] lazy compile; [R03] re-parse probe | M0 | v1#20 | U |
| H5 | Compiler, interpreter and source always present | Every build and mode links the bytecode compiler and T0 and keeps source. eval, new Function, Function.prototype.toString work in every mode. No lean runtime in full-ECMAScript mode | test262 needs them: 1,791 eval/Function( files, 80 toString files. Hermes's compiler is 2.4 of its 6.0 MB | [R14]; [R17] (rule rests on test262) | M0 rule (wk3) | R16-D9 | N |
| H6 | Value encoding | JSC NaN-box: raw cell pointers, int32 under the 0xFFFE tag, doubles + 2^49. `Value(NonZeroU64)`. Immediates as JSC: null 0x02, false 0x06, true 0x07, undefined 0x0a. Empty/hole = 0x22 (OtherTag \| 0x20), not JSC's 0: 0 stays the niche of `Option<Value>` and `JsResult<Value>`. NaN purified on boxing | No HeapNumber, no pointer decode, CSS sees raw pointers. `JsResult<Value>` is 8 B, one register | [R06] JSCJSValue.h; [P1] 8 vs 16 B; [R05] probe point 4 | M0 wk2 | v1#2 | U |
| H7 | Address space and code range | One cage per process below 2^47 (64 GiB default, configurable); avoids 0x4001_0000_0000 + 32 GiB; startup self-check. Plus one code-range reservation for JIT code and mmapped units | Payload always fits; fast in-cage test for CSS; safe on LA57 and 52-bit VA. arm64 direct calls reach only 128 MiB | [R09] value-repr; [R07] Boa #4275; [R11] | M0 wk2 (SP-VA) | v1#3 | E |
| H8 | Object header and ShapeId | 8-byte word0: u32 ShapeId, u8 gc_state, u8 kind, u16 flags; size from header; minimum object 16 B. ShapeId = offset into the shape space, O(1) to Shape. Snapshot root shapes have stable ShapeIds; other ShapeIds are run-local and never appear in AOT text. AOT guards compare RW cells filled at first execution | IC guard = one u32 compare. GC needs no shape load; barrier reads one byte. RW cells remove the need for deterministic transition trees | [R05] HEADER; [R14] Static Hermes cells; [R17] ShapeId correction | M0 wk2 | v1#4 + R16-D5 (R17) | E |
| H9 | Pages and side metadata | 256 KiB aligned pages. Side bitmaps for marks, object starts (VO) and Immix lines. Heap always parsable | JIT bakes the masks into code. CSS needs interior-pointer lookup | [R05] items 4, 6 | M0 wk2 | v1#5 | U |
| H10 | Rooting | Conservative scan of native stacks (Rust, JIT and AOT frames) after a callee-saved spill. Precise elsewhere: `Local<'s,T>`, `Global<T>`, `ScratchRoots`; lint bans `Vec`/`HashMap` of `Local`. Unit RW data visited precisely (registered by the loader) | Zero-cost locals in Rust builtins. No refcount traffic. AOT frames need no stack maps | [R05] probe S1-S7; [R09] rooting; [R07] Boa 10-18% | M0 wk2, after SP-GC | v1#6 | E |
| H11 | Barrier and atomics | One store barrier, JSC Riptide form, in T0, T1, T2, T3, baseline-AOT and optimized-AOT: `if is_cell(v) && host.gc_state <= threshold { barrier_slow(host, slot, v) } else { store }`. The slow path does the store. It remembers BLACK hosts for S1 sticky-mark generations and re-greys BLACK hosts for S2 retreating-wavefront concurrent marking (not SATB). Allocation is white; S2 ends with a stop-the-world root rescan, CSS and dirty LOS cards included. No GC point between the check and the store. GC-visible fields are Relaxed atomics; the S2 slow path adds a Release store and a SeqCst fence. Bulk barrier for copies; 512-byte cards for large arrays. Barrier version 2 in the unit header; mismatch → bytecode fallback | One barrier serves generations and concurrent marking: 4 instructions plus the cell test on aarch64. Non-cell stores need no barrier. No weak read barriers. Leyden rejects AOT code when barriers differ | [R05] BARRIER, JSC Riptide; [R11] JEP 544; H11 evidence (WebKit heap/, Riptide post, barrier model test) | API M0; slow paths S1/S2; unit field A2 | v1#7 | E |
| H12 | No address identity | Identity hash stored lazily. No raw address escapes. No Rust `Drop` on heap objects. No addresses in units | Evacuation (S3), snapshots and units stay possible | [R05] Julia ISMM'25, Alloy | M0 | v1#8 | U |
| H13 | Snapshot readiness and stable indices | External-reference table. Intrinsics in fixed order. Stable indices for isolate roots, realm intrinsics, builtins, atoms, fuses, snapshot root shapes. Deterministic snapshot layout. No raw pointers in bytecode | Snapshot, code cache and units with no redesign. Units reach heap objects only by index or cell | [R09] snapshots; [R14] snapshot indices, embedded builtins | Rules M0; index lists M1; built M4 (A1) | v1#19 + R16-D5 | E |
| H14 | Frames, calls, dispatch slots, pc→code registry | Canonical header (caller FP, return pc, callee, argc, SFI, feedback, context, offset), then registers; args reversed, padded with undefined. One layout for T0-T3 and both AOT kinds; JS→JS calls never recurse in Rust. Dispatch-slot kind {interp, baseline, opt, aot-baseline, aot-opt}. pc→code registry over JIT code and unit ranges for unwinding, Error.stack, profiler, debugger, deopt. Frame home per D21 | No adaptor frames (+11.2% jitless). OSR and deopt are copies. JIT and AOT functions call each other through one path | [R06] HTC3; [R09] PTC, profiler; [R14] shared frame, Hermes | M0 wk3; Spike B + SP-Unit (M0 month 2); Spike C (M1) | v1#10 + R16-D2 | E |
| H15 | Interpreter dispatch | Macro DSL, 3 expansions (match, sibling-call table, nightly `become`). ≤ 4 handler args: Cx, pc, frame base, accumulator. An M5c generated interpreter may be AOT-emitted and embedded | Sibling call 0.53-0.60 ns vs match 0.69-0.80 ns. Win64 passes only 4 integer args in registers | [R06] dispatch bench; [P1] Win64 dispatch | M0 wk2 | v1#11 | U |
| H16 | ICs and feedback; feedback schema = FactSnapshot schema | FeedbackVector after 8 calls. v1 slot kinds. CacheIR-like guard/action lists; mono → ≤ 4-way poly → megamorphic stub cache. IC state only in RW data; no code patching in JIT or AOT; optional embedded stub corpus. PGO profiles, hints and Warp-style snapshots share the schema; baseline-AOT ICs write it | One IC format for all tiers and code kinds. W^X-friendly (toggle pair 23 ns). Data ICs work in AOT code (Static Hermes; weval 2.77x) | [R06] HTC4; [R07] Kiesel; [R14]; [R17] | Slot kinds M0 wk4; frozen M1 exit | v1#12 + R16-D7 + R17 | E |
| H17 | Object layout | Header, properties, elements, slack-tracked inline slots. Shape: prototype, shared descriptors, weak transitions, elements kind, flags, validity cell. Constness bit + replacement watchpoints, registered as InvalidatingFuses (H31). Dictionary mode on churn | Monomorphic load = one guard + one load. T2 folds constants through watchpoints | [R06] HTC5, JSC watchpoints | M1 (frozen at M1a) | v1#13 | U |
| H18 | Elements kinds | PACKED/HOLEY × INT32/DOUBLE/ELEMENTS, one-way. DOUBLE stores raw f64 with a hole NaN. Dictionary for sparse arrays | No tag checks in numeric arrays | [R06] | M1a | v1#14 | U |
| H19 | Strings | SeqLatin1, SeqUtf16, rope (≥ 13), slice (≥ 13), thin, external. Atoms with cached hash and array-index bit. Never a Rust `String`. Snapshot atoms use H13 indices | String speed of V8/SM class | [R06] v8-string.h; [R08] | M1a | v1#15 | U |
| H20 | Builtin ABI | `fn(cx, this, args, new_target) -> JsResult<Value>`. `Args` is a frame view; `Throw` is a ZST; no `&mut` to a heap object across a JS call. AOT code reaches builtins through the isolate table or the link table | The most-copied code shape: thousands of builtins | [P1]; [R07] Brimstone handle refactor | Frozen M0 exit | v1#16 | U |
| H21 | Shape transitions under concurrent readers | ADR picks the JSC "nuke" protocol or the V8 bailout worklist. All tiers and AOT code use it | T1 emits transitions before S2b exists | [R05] S2 | ADR M1; built M5; functional before M7 | v1#17 | U |
| H22 | Isolates and realms | Isolate = one heap, one thread at a time; N realms; workers are isolates; only SAB memory shared. Unit text shared per process; unit RW data per isolate; realm values via IntrinsicRef | No cross-thread barriers on hot paths. `$262.createRealm`, `vm.createContext` | [R09] threading; [R08] | M0 wk3 (D14) | v1#18 | E |
| H23 | Limits and failure policy | Limit checks on VM and native stacks: calls, JIT and AOT prologues, recursive builtins, the 3-layer depth guard. Interrupt polls shared with safepoints and tier-up. Embedder interrupt callbacks never run JS; only an inspector session runs JS at a poll. Heap limit + callback. `panic = "abort"` in release; never unwind panics through JIT or AOT frames | No uncatchable abort on hostile input | [R09] stack/OOM/panic; [R03]; [P1] depth; [R18] interrupt probe | M0 wk3 | v1#21 | E |
| H24 | Debug and profiling hooks | Lazy position tables, kept ScopeInfo, debug bytecode copy, frame pointers everywhere, perf jitdump. Session connect pops the InspectorSession fuse (H31): deopt all, AOT frames lazily. While a session is connected, T2 and AOT code treat every interrupt poll as a heap effect point. A debugger write to a local or parameter succeeds only on a T0 frame; on a JIT or AOT frame it returns a protocol error. Breakpoints and `debugger` statements deopt to T0 before the pause. A write to a context, module or script binding first pops the DebuggerBindingWrite fuse (H31). BytecodeGen never folds bindings. Units carry pc→bytecode and position maps and a perf map | Inspector and sampler in every mode with no redesign. PROVEN facts stay unguarded: a session rewrote strict `let` and `const` locals in node (B28, B28c) and retyped a field inside a call-free loop [R18] | [R09] inspector, profiler; [R18]; V8 debug-scopes.cc | M0 rules; AOT parts A2 | v1#22 | E |
| H25 | Sandbox readiness | Off-heap pointers only in `ExternalRef<T>`. Units are trusted code (D6) | A V8-style sandbox stays possible | [R05] S4; [R09] security | M0 | v1#23 | U |
| H26 | Toolchain and Unicode | Stable 1.99.1; nightly only for opt-in `become`, fuzz, sanitizers. Unicode 17 everywhere (oxc via unicode-id-start 1.5.0, regress, ICU4X 2.3). ep9-typecheck uses tsrs's toolchain in its own workspace; its Unicode 15.1 pin never reaches the engine | No nightly lock-in. The 16 Unicode 16/17 identifier tests pass | [R06] #112788; [R02] pin leak; [R13] | M0 wk1 | v1#24 | U |
| H27 | Compilers as Cargo features | `jit` (runtime codegen) and `aot` (unit emission). The jitless build keeps T0, the compiler, the unit loader and the deoptimizer | iOS and locked-down hosts. The jitless build is a correctness oracle | [R06] jitless; [R14] | M0 (wk4) | v1#25 | E |
| H28 | Platform layer | Win64 spill trampoline, SEH unwind registration and VirtualAlloc in `ep9_platform` from M0. Plus unit mapping and unwind registration on 4 targets (A2) | Late Windows means CSS and unwind rework | [R09] Windows | M0 wk1-3 | v1#26 | U |
| H29 | Profiling and deopt contract; position-independent metadata | v1 content: slot kinds, constness and watchpoints, per-exit bytecode offset + live-value locations, MovHint-style rebuild, per-bytecode exit counters, code dependencies. Plus per exit: pc offset from code start, value locations (reg/stack/const-index/materialize), inline frame chain. One shared deopt entry. Lazy deopt by return-address swap only. Every interrupt poll (loop back-edge, function entry) is a lazy-deopt point with full metadata. Exit counters in RW data. Deoptimizer runtime in `ep9_vm` | T2 is the tier that matters. AOT text is read-only and may be signed. 12.5-17.1 deopt points per KB set unit size | [R06] HTC4; [R07] Maglev reuses deoptimizer; [R14] deopt metadata; [R16] | Draft M0 month 3; Spike C + SP-Facts; frozen M1 exit | v1#27 + R16-D4 | E |
| H30 | IR fact model with provenance | proven ⊑ predicted{source ∈ RUNTIME, PROFILED, HINTED; confidence}. Precedence RUNTIME > PROFILED > HINTED per fact kind, on a conflict. Merge rule: fused HINTED facts with no runtime counterpart (CHA, enum constant, non-null, slot pre-size) stay. Guard policy per mode in the IR. No raw heap constants: RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst. One graph builder reads bytecode + FactSnapshot. The provenance set is open; `VERIFIED{region}` is reserved for D17 (d) and built only if the D17 rule picks it [R18] | One optimizer serves JIT and AOT. Unsound TS facts never become proven | [R14] JSC AbstractValue, Warp; [R11]; [R16]; [R12] | Draft M0 month 3; frozen M1 exit | R16-D3 | N |
| H31 | Fuse registry | GuardFuse (explicit load + compare) and InvalidatingFuse (dependency → lazy deopt + dispatch-slot swap). Covers protectors, shape constness, TS-derived CHA and enum folds, the InspectorSession fuse (pops on session connect) and the DebuggerBindingWrite fuse (pops before a debugger writes a binding; deopts code that used PROVEN binding facts) | JS can patch prototypes, species and Promise.then at any time. AOT code cannot be recompiled in place | [R14] RealmFuses, 22 V8 protectors, JSC watchpoints | API frozen M1 exit; built M1-M2 | R16-D8 | N |
| H32 | Guard-failure policy per mode | JIT: DEOPT + exit counter. Hybrid: DEOPT → instrumented re-tier. Jitless: SLOWPATH or DEOPT within budget, then PIN to baseline-AOT. Never throw on hint failure. THROW only in SOUND modules at boundaries (A5, D17 c). Rows reserved for D17 (d): VERIFIED region entry-check failure → SLOWPATH from the start; interior-check failure → DEOPT with frame state; fuse pop → lazy deopt + slot swap. Never THROW | With no JIT, a deopt can repeat forever (ART "forever deopt") | [R14] ART inliner; [R11]; [R16] | Frozen M1 exit; applied A3, A4 | R16-D11 | N |
| H33 | AOT feedback rule | baseline-AOT ICs write the shared feedback schema. optimized-AOT writes only call and exit counters. Hybrid re-tiers hot optimized-AOT functions through instrumented T1, then T2. Runtime feedback overrides PROFILED and HINTED | Optimized AOT code gives no feedback for free; .NET never instruments R2R. A replayed profile can lock in slower code | [R17]; [R11] Leyden Havlak; [R14] .NET R2R | Frozen M1 exit; built A3/A4 | R17 | N |
| H34 | One Rust backend with abstract relocations | `ep9_masm` emits JIT code or unit code from one instruction stream. Kinds: Local, Stub, Runtime, LinkSlot, IcCell, RootIndex, DeoptExit, HeapConst (JIT only). No LLVM. Cranelift only for the optional T3 | Deopt and frame formats exist once. Cranelift has no deopt value maps; LLVM took > 75% of FTL compile time | [R06] Cranelift cost, FTL; [R14] one backend; [R16] | ADR frozen M1 exit; built A2/M5 | R16-D10 (refines N5) | N |
| H35 | AOT unit format | Zero text relocations. Isolate register + pc-relative RW data + link table. Link and IC cells resolved at load or first execution. ShapeId immediates only for snapshot root shapes. Header: versions, CPU features, content hash (+ optional signature). `.o` or `.ep9u` blob. Mismatch → by field (H35 S3): native-only versions → unit bytecode; build id or bytecode ABI → unit source; bad prefix, hash or signature → clean load error | Isolate-independent code. iOS needs code inside the signed app. Wasmtime refuses text relocations; Leyden rejects mismatched code | [R14] wasmtime, embedded builtins, iOS; [R11] JEP 544; [R17] RW cells | Spec v0 + header struct + golden test by M0 exit; writer/loader A2; frozen at A3 gate | R16-D6 + R17 | N |
| H36 | FP semantics in every backend | No FMA contraction. Exact IEEE-754 and ToInt32. One Rust Math implementation for all tiers and AOT. NaN purification. Differential tests. CR target, crate `ep9_math` (decided 2026-10-20) | A C/LLVM backend silently changes JS results (Porffor nbody) | [R10]; [R15] | M0 rule (wk2); FP differential CI from M1 | R16-D12 | N |
| H37 | TS hint side-table schema | ep9-owned binary schema in `ep9_hints` (zero deps). Key (start, end, kind) + decoded-text hash. dts/lib flags. Header: tsrs commit, fact-schema version, TS flags. About 20 B/node; no type strings. Files with type errors dropped. Join on the pre-lowering oxc AST. Converted to HINTED entries keyed by (function id, feedback slot) | Isolates tsrs churn (6 compile errors over 15 days), BOM/UTF-16 offset shifts and the Unicode pin. JSON was 195 B/node; type strings cost +620 MiB | [R13]; [R17] | v0 at A1h (by M4 exit); frozen at A4 start | R16-D13 + R17 | N |

**Renumbering (done once).** v1 row → H: 1→H1, 2→H6, 3→H7, 4→H8, 5→H9, 6→H10, 7→H11, 8→H12, 9→H2, 10→H14, 11→H15, 12→H16, 13→H17, 14→H18, 15→H19, 16→H20, 17→H21, 18→H22, 19→H13, 20→H4, 21→H23, 22→H24, 23→H25, 24→H26, 25→H27, 26→H28, 27→H29.

## 7. AOT foundations: M0/M1 cost and deferred parts

Stage A0 fixes the dual-mode contracts (H2, H3, H5, H8, H13, H14, H16, H29-H37) in M0/M1, for about 1-2 PM (est.).

| Contract item | H row | Fixed in | Cost |
|---|---|---|---|
| Determinism CI: two builds give identical bytecode hashes per target | H2 | M0 wk3; M0 exit gate | in total |
| CompiledUnit header fields (ABI version, source hash, TS flags, stable function ids, hint-section slot) | H1, H2, H3 | M0 wk3 | in total |
| Rule: compiler, interpreter and source always present | H5 | M0 wk3 | in total |
| Dispatch-slot kinds + pc→code registry API | H14 | M0 wk3 | in total |
| ShapeId rule | H8 | M0 wk2 | in total |
| Stable-index rules | H13 | Rules M0; index lists M1 | in total |
| FactSnapshot = feedback schema (slot kinds as FactSnapshot v0) | H16 | M0 wk4; frozen M1 exit | in total |
| Position-independent deopt metadata | H29 | Draft M0 month 3; frozen M1 exit | in total |
| Fact lattice + ref nodes | H30 | Draft M0 month 3; frozen M1 exit | in total |
| Fuse API | H31 | Frozen M1 exit | in total |
| Guard-policy ADR and AOT feedback rule | H32, H33 | Frozen M1 exit | in total |
| Relocation-kind ADR | H34 | Frozen M1 exit | in total |
| Container spec v0 + header struct + golden test | H35 | M0 exit | in total |
| FP rule: `clippy.toml` + `xtask fp-check` stub | H36 | M0 wk2 | in total |
| Hint-section slot only (schema v0 comes at A1h) | H37 | Slot M0 wk3 | in total |
| SP-Unit inside Spike B: 10 opcodes as a unit blob (load, call, unwind, GC scan, version fallback) | H14, H35 | M0 month 2 | +0.25 PM (est.) |
| SP-Facts inside Spike C: lattice + ref nodes, PI metadata, one kernel from a PROFILED FactSnapshot file | H16, H29, H30 | M1 | +0.25-0.5 PM (est.) |
| **Total** | | M0-M1 | **about 1-2 PM (est.)** |

| Deferred part | Stage | When |
|---|---|---|
| Unit writer/loader (.o + .ep9u), link table, unwind on 4 targets | A2 | After M1 exit; due at M5 start |
| `ep9c` driver | A1 → A3/A4 | `--bytecode` at M4; `--baseline` at A3; `--opt` at A4 |
| PGO capture (`--profile-out`, `.ep9prof`) | A3 | With M5 |
| AOT emit: baseline-AOT, optimized-AOT | A3, A4 | With M5 (K-A3), with M7 (K-A4J) |
| Hybrid re-tier | A4 | With M7; kept only if K-A4P passes |
| Hint consumers (oracle and join come earlier, at A1h; D18) | A4 | With M7; kept only if K-A4H passes |
| Sound typed code | A5 = M10 | Only if the D17 rule at M7 exit picks (d) or (c) |
| iOS packaging | A3 | Only if D16 = yes |

Details: [05-aot.md](./05-aot.md) (stages, gates, kill criteria), [09-milestones.md](./09-milestones.md) (calendar).

## 8. Crate layers

```
 binaries   ep9 (ep9_cli; JIT build or jitless build)                ep9c (`aot`)
                 |                                                      |
 runtime    ep9_runtime   ep9_node   ep9_inspector   ep9_napi           |
                 |                                                      |
 facade     ep9 (unstable; + unit loading API)                         ep9_aot (`aot`): module graph,
                 |                                                      |   .ep9prof, .ep9h, hot set
 compilers  ep9_baseline (T1 | baseline-AOT)      ep9_opt (T2, [T3] | optimized-AOT)
                 |                                                      |
 codegen    ep9_masm (`jit` finalize | `aot` finalize)        ep9_unit (loader: default | writer: `aot`)
                 |
 engine     ep9_vm: T0, ICs, deoptimizer, fuse registry, dispatch slots, pc->code registry
            ep9_builtins_*   ep9_regexp   ep9_intl   ep9_temporal   ep9_numconv   ep9_snapshot
                 |
 formats    ep9_frontend (only oxc importer) -> ep9_bytecode    ep9_facts (FactSnapshot, .ep9prof)
            ep9_hints (.ep9h reader/writer, zero dependencies)
                 |
 base       ep9_heap   ep9_value   ep9_platform

 separate workspaces, never in the engine graph:
   tools/ep9-typecheck  -> tsrs crates by path + ep9_hints (path)    third_party/tsrs -> tsrs CLI
```

Full crate table, features and binary composition: [08-repo.md](./08-repo.md).

## Appendix: v1 merge choices C1-C14

| # | Choice (one line) |
|---|---|
| C1 | Speed order: contracts, interpreter parity, baseline JIT, mid tier. Compliance, runtime and AOT run in parallel lanes |
| C2 | Design canary: M0 richards and deltablue ≥ 0.6x QuickJS; M1 Octane-7 ≥ 1.0x; M2 parity |
| C3 | M2 hard gate on the 7-kernel geomean; per-kernel floor 0.8x with a root-cause ADR and parity debt cleared by M5 exit |
| C4 | RegExp: RX1 (irregexp bytecode port) starts in M2; its kernel gate is due at M5 exit. RX2 (native backend) is in M5 |
| C5 | Front-end output is a GC-free, serializable CompiledUnit; `finalize()` on the isolate thread. **Changed:** it is also the unit bytecode format |
| C6 | oxc fallback: upstream PR first, then ≤ 3 local patches through `oxc.commit`; own parser only if a P-gate trips |
| C7 | Windows x64 tier-1 from week 1; the JIT reaches Windows in M5; `ep9 check` unix-only until SP-tsrs-win passes |
| C8 | GC generations through S1 sticky mark bits in M1 |
| C9 | Shape-transition ADR in M1; S2b built in M5 and functional before M7; pause and TSan gates at M8 exit |
| C10 | Spec layer split into `#![forbid(unsafe_code)]` crates by test262 directory; hot core in `ep9_vm`; fat LTO |
| C11 | tsrs_cli is built inside tsrs's own workspace and spawned. **Changed:** the sync also builds ep9-typecheck |
| C12 | The runtime uses the unstable `ep9` facade; `node:*` hot paths may call `ep9_vm` internals behind a feature |
| C13 | Node-API in runtime beta (M6), in parallel with the JIT lane |
| C14 | Lazy compile L0 → L1 → L2. **Changed:** plus the AOT eager rule (`ep9c` compiles every function) |

## Changes vs plan v1

| Area | Change |
|---|---|
| Goals | G1, G9, G10 extended; G11-G15 added; N5 refined; N10-N13 added |
| Diagram | v1 §3 redrawn for dual mode: build path, shared compilers, unit format, fact sources |
| Hard decisions | v1 §4 27 rows → H1-H37: 10 new (H3, H5, H30-H37), 14 extended (H23 since 2026-10-20), 13 unchanged; renumbered once |
| New tables | Fact sources, deployment modes, mode × code kind, AOT-foundation cost |
| Crates | Deoptimizer runtime moves to `ep9_vm`; `ep9_jit_baseline` → `ep9_baseline`, `ep9_jit_opt` → `ep9_opt`; new `ep9_facts`, `ep9_hints`, `ep9_unit`, `ep9_aot`, `ep9c` |
| Unchanged | Speed targets G2-G4, value encoding, object model, builtin ABI, all v1 rows marked U |
| Decisions 2026-10-20 | H11: JSC Riptide barrier; S2 is retreating wavefront, not SATB (GC algorithm change). H6: empty/hole bits stated. H23, H24, H29-H32: debugger, interrupt-poll and D17 (d) reservations [R18]. H35: mismatch outcome by field. H36: CR target, `ep9_math`. ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)) |

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
| [R09] | [../research/09-critic.md](../research/09-critic.md) (wins among R01-R08) |
| [R10] | [../research/10-js-aot-precedents.md](../research/10-js-aot-precedents.md) |
| [R11] | [../research/11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R12] | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R13] | [../research/13-tsrs-type-oracle.md](../research/13-tsrs-type-oracle.md) |
| [R14] | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R15] | [../research/15-aot-perf-reality.md](../research/15-aot-perf-reality.md) |
| [R16] | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R17] | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) (wins over [R16]) |
| [R18] | [../research/18-sound-regions.md](../research/18-sound-regions.md) |
| [P1] | Plan v1 local probe (session-local); re-checked by SP-Value, M0 week 3 depth tests and SP-Dispatch |
