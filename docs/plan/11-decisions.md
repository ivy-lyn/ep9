# ep9 plan: decisions

- The user fixed 8 decisions (§1).
- 21 decisions (D1-D21) were open. On 2026-10-20 the user said: "Don't ask me questions. Deepresearch and descide yourself". The agent team decided all 21 that day ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)).
- Each row now has a Decision and a Kind. "Decided now" is final until its re-check point or an owner override. "Decided rule" fixes a default and a written rule. Gate data at the named point picks the outcome, and nobody is asked a question.
- The orchestrator applies each rule and logs the data and the result in the decisions log. The human owner reviews at will and can override any entry.
- D15-D21 come from the dual-mode goal: one engine, three modes (JIT, AOT-jitless, hybrid), one optimizer. D17 gains option (d), automatic VERIFIED regions [R18].
- The AOT choices wait for data where the plan says so: D17 at M7 exit (after SP-RegionCoverage at A1h, and SP-TypedBound and SP-RegionBound at M7), and D18 by its off-ramp and K-A4H.
- No AOT part leaves v1 except by its kill rule. The orchestrator applies the rule at the points in §2.

IDs used here: D1-D21; H1-H37; G9, G10, G12, G14, G15; N10; M0-M10, M1a; A1h, A3, A4, A5; T0-T3; M5c; RX1, RX2; P1-P5; S4; FV1-FV3; K-A3, K-A4J, K-A4P, K-A4H, K-A5pre, K-A5, K-A5v-cov, K-A5v-bound, K-A5v; SP-tsrs, SP-tsrs-win, SP-HintPrecision, SP-TypedBound, SP-RegionCoverage, SP-RegionBound, SP-Dispatch, SP-VA, Spike B; PROFILED, HINTED, SOUND, VERIFIED; A5v; THROW; RK5, RK28, RK34, RK36, RK38, RK43.

Evidence tags: [R04] [R05] [R06] [R07] [R08] [R09] [R10] [R11] [R12] [R13] [R14] [R15] [R16] [R17] [R18] [P1] [TSRS]. Paths are in the table at the end.

---

## 1. Fixed by the user

| # | Decision | Where the plan answers it |
|---|---|---|
| 1 | Product: an embeddable engine and a Node/Deno-like runtime, for production use | [02-architecture](./02-architecture.md), [07-runtime](./07-runtime.md) |
| 2 | Scope: full ECMAScript (test262), incl. Intl and Temporal | G1; [06-object-model-and-spec](./06-object-model-and-spec.md) |
| 3 | Priority: speed | Speed order of milestones; [09-milestones](./09-milestones.md) |
| 4 | Question: does tsrs fit as the JS/TS parser? | No for the engine parser; yes for `ep9 check` and the offline type oracle; [01-front-end](./01-front-end.md) |
| 5 | GC: user unsure; suggested "hybrid like V8" | Hybrid like JSC (generational, concurrent, non-moving first); [03-gc](./03-gc.md) |
| 6 | `unsafe` Rust is allowed | Contained in named crates; [08-repo](./08-repo.md) |
| 7 | Out of scope: stable embedding API design, a WASM build target of ep9, no_std | N1, N2 |
| 8 | Dependency style: unpublished source deps in `third_party/<name>/` (git-ignored), pinned by `third_party/<name>.commit`, fetched by `scripts/sync-<name>.sh` | [08-repo](./08-repo.md) |

The user's core idea (JIT and AOT in one engine, AOT powered by TypeScript types) is the top goal, not an open decision. [05-aot](./05-aot.md) gives the honest answer. All other open items were decided on 2026-10-20 (§3).

---

## 2. How the decisions depend on each other

```
D3  team (agents) ──────────────────────> PM sizes only; no calendar

D15 AOT product claim ──┬──> D17 sound typed code (rule) ─> M10 = A5 (c|d), FV3, K-A5, K-A5v*
                        └──> D18 hint investment ─────────> A1h, A4 hint consumers, K-A4H

D11 JIT-free mode ──> D16 iOS / jitless near-term ──┬──> D2 platforms (iOS arm64)
                                                    └──> A3 gate timing (M5 exit or M8 exit)

D13 baseline tier ─────> A3 (T1 is the baseline-AOT emitter)
D19 AOT compile scope ─> A3, A4 (hot set, payload size)
D20 class fields ──────> H3 (TS lowering in BytecodeGen, header flags, cache key)
D21 frame home ────────> H14, Spike B, M5c
D9  C++ allowed ───────> T3 / A5 backend (no LLVM; Cranelift stays the T3 option)
D6  sandbox ───────────> unit trust model (H25, H35)
```

Re-check calendar (every item was decided on 2026-10-20):

```
M0 month 2 ...... D21 Spike B places JIT frames
M0 exit ......... D3 throughput check (PM per week vs estimates)
M1a ............. D6 last cheap point (owner request only)
M2 exit ......... D13 rule (T1 stays while D11 = yes); D1 P1, P2, P4
before M4 ....... D5 Wasm trigger count
M4 start ........ D4 node/test exclusion list
A1h (by M4 exit)  D18 off-ramp (SP-HintPrecision); K-A5v-cov (SP-RegionCoverage) for D17
A3 SIZE-REPORT .. D19 size switch
M5 exit ......... K-A3 pre-check: jitless top tier (D11 rule); D16 rule; D9 RX1 debt rule
M7 exit ......... D17 rule (K-A5pre, K-A5v-bound)
M8 exit ......... AOT ship review: K-A4J, K-A4P, K-A4H; D15 (t1): failing A4 parts leave v1; D18 final; a K-A4J trip stops D17 (d)
```

**Re-check points.** Gate data comes back to the orchestrator, not as a question. It applies the written rule and logs the data and the result. The owner may override.

| When | Data | Rule applied |
|---|---|---|
| A1h (by M4 exit): SP-HintPrecision + SP-RegionCoverage | `any`, class-receiver and slot-found shares per HINT-SET group; hot share in finder-v1 regions | D18 off-ramp; K-A5v-cov (D17 input) |
| K-A3 pre-check (M5 exit) | baseline-AOT vs T0 on JITLESS-SET | jitless top tier (D11 rule); D16 rule |
| M7 exit | SP-TypedBound (K-A5pre); SP-RegionBound (K-A5v-bound) | D17 rule |
| A3 gate (M8 exit) | FV1, K-A3, SIZE-REPORT | jitless top tier; D19 size switch |
| M8 exit: AOT ship review | K-A4J, K-A4P, K-A4H; AOT-vs-own-JIT report; latency and G15 report | which AOT parts ship in v1 (D15 t1); D18 final; K-A4J trip stops D17 (d) |

---

## 3. Decisions (all decided 2026-10-20)

### 3.1 D1-D14 (from plan v1)

| ID | Question | Options | Decision (2026-10-20) | Kind | Re-check point | Cost of the other options | Affects |
|---|---|---|---|---|---|---|---|
| D1 | Final parser path | A tsrs as is. B tsrs fork. C oxc pinned. D own parser now. E: C now, local oxc patches (≤ 3), D only on a P-gate, tsrs out of process | **E** | Decided now | P1, P2, P4 at M2 exit; P3, P5 at M4 exit | B: +6-12 PM, a permanent fork, `plain-ptrs`, Unicode patch; run SP-tsrs first. The type oracle stays separate unless the fork uses `plain-ptrs` and Unicode 17; the hash check is still needed. D now: +6-12 PM before M1; the perf slice moves later [R04][R09] | H1, H26; P1-P5; M0-M1 |
| D2 | v1 platforms. Must `ep9 check` work on Windows in v1? | Unix only; + Windows x64; + Windows arm64; + iOS arm64 (AOT-jitless) | macOS arm64, Linux x64/arm64, Windows x64, all tier-1. A gate counts as passed on Windows x64 only after a run on Windows; a cross-build alone does not count. `ep9 check` and `ep9c build --hints` are unix-only until SP-tsrs-win passes (not run yet). Windows arm64 after v1. iOS arm64 only if D16 = yes | Decided now | SP-tsrs-win: the first Windows run, and again at A1h | Drop Windows: saves about 1-3 PM (Win64 trampoline, SEH, VirtualAlloc); the runtime cannot claim Node-class reach. Require `ep9 check` on Windows: SP-tsrs-win must pass, else ep9 carries tsrs patches that upstream does not test [TSRS]. Add iOS: +2-4 PM (est.) in A3, through D16; iOS also limits the address space, so the cage must shrink to about 4-8 GiB (est.) | H7, H28, G10; A3 |
| D3 | Team and AI share | 2-3, 5, 6, 9 or 13 seniors, plus agents; AI agents only | **AI agents, led by an orchestrator agent.** Agents fill the six plan roles. The human owner reviews at will and can override any decision. Required reviews (10-quality §6) follow ADR-P1: an independent reviewer agent plus a Codex adversarial review, never the author alone. PM stays as the size of work. Calendar months (PM ÷ FTE) do not apply | Decided now | M0 exit: compare PM done per week with the PM estimates; re-plan the ranges if they differ by more than 2x (est.) | Human team of 6 seniors + agents: v1 at about 57-100 months; 5: 65-117; 9: 34-60; 13: 24-41, with less than linear gain on M7 [R07] | all calendars; RK28, RK43 |
| D4 | Node compatibility depth. Also approves the node/test exclusion list | ESM only; + CJS; + V8 stack API; + Node-API; + TS legacy decorators (`experimentalDecorators`, `emitDecoratorMetadata`) | ESM + CJS + V8 stack API in M4. Legacy decorators through oxc_transformer, only in files that use them (M4); those files get no hints in v1. Node-API in M6. node/test exclusions: one ADR list; each entry names one reason: V8 internals or V8 flags; a feature outside D4, D5 or N1/N2; a platform outside D2. The orchestrator approves the list at M4 start | Decided now | M4 start (exclusion list) | No Node-API: drop `ep9_napi`, saving 3-6 PM; oxc-parser, rolldown and lightningcss will not run; keep the Global, weak and external-memory APIs anyway. No legacy decorators: NestJS, TypeORM and Angular .ts code fails to run | H3; M4, M6 |
| D5 | WebAssembly in the runtime | None; wasmtime JS API; own Wasm tiers | wasmtime JS API after v1. Signal and VA coexistence rules apply now. The M6 app gate uses an app with no `.wasm`. JetStream runs with `cli.js --no-prefetch`, because its default prefetch inflates files with a WebAssembly zlib | Decided rule | Before M4: move Wasm into v1 only if ≥ 2 of the M6 app-gate candidates need WebAssembly, or ≥ 5% (est.) of the pinned npm top-1,000 tarballs contain a `.wasm` file. Else after v1 | In v1: +3-6 PM on RT before M6. `vite build` becomes the M6 gate, because vite compiles WebAssembly at load [P1]. WasmGC interop means two heaps with cross-heap cycles | H7, H28; M4, M6 |
| D6 | Untrusted-code sandbox | None; V8-style sandbox | None in v1. The `ExternalRef` rule applies from M0. Units are trusted code, like native addons | Decided now | M1a (object layout freezes): the last cheap point; only an owner request re-opens it | Yes: pointer tables, cage-relative ArrayBuffer pointers, CFI; +6-12 PM; due before the M1a object layout freezes. With a sandbox, only signed units or bytecode-only units may load [R05][R09] | H25, H35, H17; S4; RK36 |
| D7 | Proper tail calls | Accept the gap (V8, SM); implement (JSC 35/35) | Accept the gap: 35 tests, 0.07% [R09]. `TailCall` opcode reserved; no Rust recursion. The H14 ADR (M0 wk3) shows that frame reuse fits the layout, AOT frames included | Decided now (logged 2026-10-20) | H14 ADR (M0 wk3) | Implement: +2-4 PM; frame reuse in every tier and both AOT code kinds; stack traces lose frames | H14; M5, M8 |
| D8 | Toolchain | Stable only; nightly `become` by default | Stable 1.99.1. The nightly build is opt-in | Decided now (logged 2026-10-20) | Each toolchain bump (H26) | Nightly default: SP-Dispatch measured `become` at 0.982x the time of stable sibling calls (geomean, aarch64), and stable sibling calls already meet the gate (0.998x cgoto.c; 1.365x faster than match). Gain: guaranteed TCO, so debug builds work. Cost: toolchain risk | H15, H26 |
| D9 | Is C++ allowed? | Pure Rust; irregexp FFI as a stopgap; LLVM AOT backend | Pure Rust: no irregexp FFI, no LLVM AOT backend. Cranelift stays the option for T3 (shared by JIT and AOT emit). C only in test and benchmark references that never ship | Decided now | RX1 gate at M5 exit. A miss becomes RegExp debt (09-milestones §5 rule 4), not FFI | FFI: RegExp kernel parity sooner and less CORE load, but C++ on 4 CI targets [R08]. LLVM AOT: a second backend with its own deopt and frame formats, FMA risk (H36) and a C++ toolchain | RX1, RX2; H34, H36; M9, M10 |
| D10 | Intl data in the default build | Full baked data (about 4.94 MB); lean build + data blob | Full data in the `ep9` binary. A lean feature for embedders [R08] | Decided now | SIZE-REPORT (G15) from A1 | A lean default needs a data loader and a way to ship the blob | M3; SIZE-REPORT |
| D11 | Is a JIT-free mode required (iOS, locked-down hosts)? | yes / no | **Yes**: AOT-jitless mode ships in v1. T0 gates are also measured in the jitless build. Gate data never removes the mode; it picks its top tier: baseline-AOT if K-A3 passes, else M5c if it passes test262 equality, else T0 | Decided now | K-A3 pre-check (M5 exit), A3 gate, M8 AOT ship review: they pick the top tier only | No: T0 tuning can stop once T1 ships. AOT-jitless mode (A3 jitless path, FV1, K-A3, K-A4J) leaves v1; A1 and hybrid stay. D16 becomes moot | H27, G9, G12; A3; D16 |
| D12 | Upstream contacts | Who owns the oxc track? Ask tsrs for an "engine mode"? | No upstream contact. The front-end lead agent owns the oxc track. Agents make no public posts. Each needed oxc change is a local patch (≤ 3, P1) plus an upstream-ready draft in the repo; the owner may file the drafts at will. No tsrs request (its port rule forbids it). The oracle uses the tsrs public API (≤ 2 local patches, est.) [R13] | Decided now | P1 at M2 exit | With an oxc contact: the same plan; local patches weigh less and P1 trips later | P1; A1h; RK5, RK38 |
| D13 | Baseline tier | Build T1; skip straight to T2 (the LibJS 2026 design) | **Build T1.** It is also the A3 baseline-AOT emitter and the M5c generator | Decided rule | M2 exit: if D11 = yes, build T1. Only if D11 is no by then: skip T1 when `node --no-sparkplug` scores ≥ 0.97x `node` (est.) on the JetStream M7 list and the TS workload, re-measured under the 10-quality §3 speed-gate rule; else build T1 | Skip: saves 18-36 PM. On the TS workload, Maglev without Sparkplug is within about 3-5% (0.62 vs 0.64 s [P1]). The loss below T2 thresholds is not measured. No fast tier from M2 until M7; Windows/unwind risk moves into T2 [R06]. Also no baseline-AOT and no M5c: the jitless top tier stays T0, so FV1 fails | M5, A3, H33; K-A3 |
| D14 | Worker heap model | Isolate per thread; shared heap | Isolate per thread | Decided now (logged 2026-10-20) | H22 ADR (M0 wk3) | Shared heap: a cross-thread safepoint protocol and cross-thread barriers [R09]. After M0 wk3 it needs an ADR amendment of H11, H22 and H23 | H11, H22, H23 |

### 3.2 D15-D21 (new for dual mode)

| ID | Question | Options | Decision (2026-10-20) | Kind | Re-check point | Cost of the other options | Affects |
|---|---|---|---|---|---|---|---|
| D15 | Product claim for AOT. Also: v1 timing if A4 trails M7 | (a) AOT = startup and warm-up, jitless speed, predictable latency, no JIT compile CPU, possibly smaller total shipped size; the JIT owns general peak. (b) (a) + "beat V8 at peak on sound typed numeric class code": FV3, TYPED-KERNELS only; it names the mechanism that passed FV3 under the D17 rule (opt-in subset (c) or automatic VERIFIED regions (d)). (c) "AOT with TS types beats V8 in general". Timing: (t1) v1 waits for the A4 gates. (t2) v1 ships JIT mode, A1 and the jitless top tier on M7 timing; A4 follows in v1.x | **(a) now; (b) only as the gated research stage A5 = M10; (c) rejected (N10).** Timing: **(t1)**: v1 waits for the A4 gates; A4 parts that fail K-A4J, K-A4P or K-A4H leave v1 by their kill rule. (t2) only on an owner request for an earlier v1 | Decided now (claim and timing) | M8 AOT ship review (which AOT parts ship, by K-A4J, K-A4P, K-A4H); an owner request for (t2) | (b) as a v1 goal: A5 moves before M8 and competes with M7 for review capacity; it needs a D17 build choice and a T3-class backend (M9). (c): no measured system supports it; the architecture would chase a peak gain that guard-removal studies put at 2-8% [R12][R17]; automatic regions cover only 0-11% of CPU time in real libraries [R18]. (t2): optimized-AOT, hybrid mode and HINTED facts wait for v1.x, so v1 ships without the top goal's main AOT parts (§1). Under D3 a calendar slip has no fixed cost, so (t2) buys nothing by default | N10, G14; FV1-FV3; M8, M10 |
| D16 | iOS / jitless AOT as a near-term target | yes / no | **No**: no iOS before v1; A3 gate at M8 exit | Decided rule | K-A3 pre-check (M5 exit) and A3 gate: add iOS arm64 as a v1.x target only if baseline-AOT ≥ 1.6x T0 on JITLESS-SET and SP-VA passes on an iOS arm64 device with a cage of ≤ 4 GiB. Else stay no | Yes: iOS arm64 becomes a target (D2); the A3 gate moves to M5 exit; units ship as .o first; +2-4 PM (est.); iOS simulator CI from M5 (est.); a small cage, because iOS limits the address space | D2, H7, H28, H35; A3 |
| D17 | Sound typed code (A5) | (a) none. (b) Static Hermes style: Flow-like exact objects, nominal classes, throwing casts. (c) TS-syntax nominal-class subset, checked by tsrs plus ep9 boundary checks; opt-in per module, THROW at SOUND boundaries, outside test262 mode. (d) Automatic VERIFIED regions: ep9c finds and proves regions in normal code; each fact has a static proof, a check or a fuse; failure runs generic code; no opt-in, no edits, no THROW; legal in test262 mode [R18] | (b) rejected now. At M7 exit the rule picks one: (1) K-A5v-cov passed and region code ≥ 1.1x A4 code inside regions → (d), real apps and kernels. (2) Else region code ≥ 1.3x A4 code on ≥ 2 of 4 TYPED-KERNELS → (d), kernels only. (3) Else K-A5pre ≥ 1.15x → (c), FV3 vehicle only. (4) Else (a); M10 dropped. K-A4J trips → (d) stops | Decided rule | A1h: K-A5v-cov (SP-RegionCoverage). M7 exit: K-A5pre (SP-TypedBound), K-A5v-bound (SP-RegionBound). M8 exit: K-A4J | (d) full: about 16-33 PM (est.; 10-21 PM if T2 already budgets the H31 and H17 work). (c): M10 prototype 12-24 PM after M7; `ep9c build --sound`; THROW only at SOUND boundaries (H32); a dialect that users then depend on. (b): users must rewrite TS into a Flow-like dialect. (a): no FV3; M10 dropped | M10 = A5; FV3, K-A5pre, K-A5, K-A5v-cov, K-A5v-bound, K-A5v; H30, H31, H32 |
| D18 | TS-hint investment before measurement | (a) full hint pipeline in the optimizer early. (b) cheap oracle now (A1h), consumers only in A4, kill by K-A4H. (c) skip TS types | **(b)** | Decided now; off-ramp by rule | A1h: class-receiver share < 5% and slot-found < 5% on ≥ 2 of the 3 real-app groups → (c) for HINTED consumers before A4; ep9-typecheck stays only if K-A5v-cov passed; `ep9 check` stays. M8 exit: the K-A4H keep rule | (a): hint consumers are built before any gain is measured, so their 3-6 PM is spent even where K-A4H would kill them. (c): no ep9-typecheck and no HINTED facts; saves A1h (1-2 PM) and the A4 hint work (3-6 PM); `ep9 check` stays; A5v loses its finder input [R18] | H30, H37; A1h, A4; K-A4H |
| D19 | AOT compile scope | (a) all functions. (b) PGO hot set. (c) baseline-AOT for all reachable functions + optimized-AOT for the hot set | **(b) for hybrid; (c) for jitless** | Decided rule | A3 SIZE-REPORT: if (c) gives unit text > 8x the bytecode of compiled functions, jitless baseline-AOT covers only functions that ran in the training run; the rest stays bytecode for T0 | (a): per-app payload grows about 10x (baseline and mid-tier native code is 6-19x the bytecode); simpler driver. `ep9c --all` keeps (a) as a flag | A3, A4; ep9_aot; SIZE-REPORT; RK34 |
| D20 | TS class-field and decorator semantics when ep9 strips types | (a) always [[Define]], like node strip-types. (b) follow tsconfig (`useDefineForClassFields`, `experimentalDecorators`) when present, else [[Define]] | **(b)** | Decided now | H3 ADR (fields M0 wk3; built M4) | (a): code that tsc compiles with [[Set]] (4 of 6 measured codebases) gets other shapes and other behavior. Example: base-class setters stop running for fields | H3, H37; M4; cache key |
| D21 | Interpreter frame home (the AOT emitter must match it) | (a) VM register stack + native control stack (Hermes model). (b) generated native-stack interpreter now | **(a)**, with the canonical frame layout kept so M5c can switch later | Decided now; JIT-frame placement by rule | Spike B (M0 month 2): keep native-stack JIT frames if one T0→JIT→T0 round trip costs ≤ 18 release-expansion dispatches of the loop kernel (about 6-10 ns at SP-Dispatch's 0.36-0.57 ns per dispatch); else JIT frames also live on the VM register stack (one VM stack, H14 ADR). (b) is not a Spike B outcome | (b): deopt and OSR become plain frame copies. But the interpreter then needs a code generator before the M2 gate. The plan allows codegen infra only after M1 exit, so this work moves earlier on CORE | H14; Spike B; M5c |

---

## 4. Evidence for D15-D21

**D15 (product claim).**
- No measured JS AOT system beats V8 on general code. Static Hermes typed native is 1.5-3.2x slower than V8 full (geomean about 2.1x) [R15].
- Its gap is implementation debt (no native ints, 5 inline slots, out-of-line array loads), not a ceiling [R15][R17].
- scriptc ran nbody about 1.15x faster than V8 at peak, but it refuses eval and changes JS semantics [R10][R17].
- Graal PGO native-image beat its own JIT (0.89-0.91 vs 0.92 of C2), with a profile trained on the same benchmarks [R11][R17].
- Hello-world startup and RSS gaps are runtime bootstrap effects, not AOT effects [R17].
- Timing: A4 needs T2, so the A4 gates fall at M8 exit. Under D3 a calendar slip has no fixed cost, so the timing stays (t1): v1 waits for the A4 gates, and A4's own kill rules drop the parts that fail. (t2) would ship v1 without HINTED facts, optimized-AOT and hybrid mode (critic change, 2026-10-20).

**D16 (iOS / jitless near-term).**
- Third-party iOS apps cannot JIT, except browser engines with BrowserEngineKit entitlements [R11][R14].
- So iOS units must be static objects (.o) linked into the signed app [R14].
- Baseline-class AOT gives 1.6-2.8x its interpreter (Hermes native 1.6-2.0x, weval 2.77x) [R14]. That is the FV1 basis.
- If unsure, A1 ships first: cached bytecode gives 2-10x on large bundles with no machine code [R10][R17].

**D17 (sound typed code).**
- Sound subsets change semantics that test262 forbids: throwing casts and out-of-bounds throws [R12][R16].
- Static Hermes' `-typed -parse-ts` path failed even on a 2-line TS file [R15]. Users write TS, not Flow.
- Gains vary: StrongScript inside 2015 V8 gave -0.9% to +22%; Safe TypeScript checks cost 6.5-15% [R12].
- SP-TypedBound measures the headroom on ep9's own T2 before anyone builds A5 (K-A5pre) [R17].
- Automatic regions are sound in full ECMAScript if each fact has a static proof, a check or a fuse, and failure runs generic code. They never THROW, so test262 mode stays legal [R18].
- Their coverage in real code is small: 0-11% of CPU time in tsc, hono, zod and date-fns; 24-39% in the vscode diff engine. Only 0-6.2% of member accesses sit in a region on a qualifying receiver [R18].
- Estimated gain (low confidence): real apps +0.2-3.7% end to end in AOT-jitless mode vs A4; kernels up to +16-27% [R18].
- SpiderMonkey shipped a related run-time design (Type Inference) and removed it; Warp won on real pages and lost on Octane and Kraken [R18].

**D18 (hint investment).**
- The oracle prototype works: 99.94% join on 556,724 nodes, 0.87 s for 18 MB with 16 checkers [R13].
- The codegen gain is unmeasured. Guard-removal studies give 2-8% [R12][R17].
- "Insufficient feedback" was 33% and field-type dependencies about 10% of tsc deopt events. These are events, not time [R12][R17].
- Slot hints apply to 64% of member accesses in typed AWFY code but 0.2% in TypeScript's src/compiler [R13].
- The join was measured before oxc's TS transform, so TS lowering moves into BytecodeGen (H3) [R17].

**D19 (compile scope).**
- Baseline and mid-tier native code is 6-19x the bytecode bytes; TurboFan with inlining reaches 28.6x [R14]. Static Hermes native tsc grew from 26.3 to 69.8 MB [R15][R17].
- Default (c) for jitless departs from [R14], which advised a hot set even for baseline-AOT. [R16] advises (c) for jitless cold-code speed. The A3 size gate (unit text ≤ 8x bytecode) bounds it.
- Android 6 full AOT made installs take minutes; Android 7 moved to profile-guided AOT. ART profiles about 20% of methods [R11][R14].
- A replayed Leyden profile gave a 31% faster first iteration but a 20-40% slower steady state [R11]. So runtime feedback always overrides PROFILED facts.

**D20 (class-field semantics).**
- 4 of 6 measured codebases compile class fields with [[Set]]; node strip-types uses [[Define]] [R12].
- The same source gives different own keys: [] under [[Set]] vs ["z"] under [[Define]] [R12].
- oxc_transformer builds [[Set]] field stores with dummy spans, so ep9 lowers these in BytecodeGen [R17].
- The mode is part of module identity: CompiledUnit header, .ep9h header and cache key [R16].

**D21 (frame home).**
- Hermes runs interpreter, JIT and AOT-native frames on one register-stack layout (SHStackFrame) [R14].
- Spike B keeps native-stack JIT frames if a T0→JIT→T0 round trip costs ≤ 18 release-expansion dispatches of the loop kernel (est.).
- A generated native-stack interpreter (M5c) can later be emitted by the AOT emitter and embedded at engine build time [R14][R16].
- SpiderMonkey went from 31 to 52 on Speedometer with a generated baseline interpreter [R06].

---

## Changes vs plan v1

- D15-D21 added.
- Restated for dual mode: D2 (iOS through D16), D3 (6 seniors; variants 5/6/9/13), D6 (units are trusted code), D9 (no LLVM AOT backend), D11 (yes = AOT-jitless mode ships), D12 (oracle on the tsrs public API), D13 (T1 is also the baseline-AOT emitter).
- D4 notes that legacy-decorator files get no hints in v1. D7 covers AOT frames.
- Every decision now has a deadline and an "Affects" column. A dependency diagram and an evidence section for D15-D21 are new.
- New re-open points bring kill data back to the user (§2). D15 gains a v1 timing option for the case where A4 trails M7.
- Unchanged in substance: D1, D4, D5, D7, D8, D10, D14.

## Changes on 2026-10-20 (decisions)

- All 21 items decided by the agent team under the user's instruction ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)). The Decision, Kind and Re-check point columns replace Default assumed and Deadline.
- D3 = AI agents led by an orchestrator. PM stays as a size; calendar months do not apply. Required reviews follow ADR-P1.
- D17 gains option (d), automatic VERIFIED regions [R18], and a rule at M7 exit. (b) is rejected.
- D15 (b) is worded without a mechanism. D15 timing stays (t1) (critic change). New kill rules: K-A5v-cov, K-A5v-bound, K-A5v. K-A5 keeps only its FV3 clause.
- Re-open points become re-check points. The orchestrator applies the written rules there.

## Changes on 2026-10-21

- D21 clarified ([decisions-log §7](../decisions-log.md#7-2026-10-21-h15-dispatch-on-x86-64-sp-dispatch-reopen), D21-UNIT): the threshold (§3.2, §4) for Spike B counts release-expansion dispatches of the SP-Dispatch loop kernel on the same host, not "sibling-call dispatches". The number 18 does not change.

---

## Evidence tags

| Tag | Path |
|---|---|
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
| [P1] | Plan v1 local probe (session-local); re-checked by the named M0 spike or review (D5 review for the vite case; M0 week 1 shell runs for the Maglev/Sparkplug timing) |
| [TSRS] | `third_party/tsrs` at `tsrs.commit` |
