# ep9 plan: AOT and dual mode

- ep9 is one engine with three modes (JIT, AOT-jitless, hybrid) and one optimizer. The dual-mode contracts are fixed in M0/M1 for about 1-2 PM (est.).
- The JIT owns peak speed on general JS. No measured JS AOT system beats V8 there, so ep9 makes no general "AOT beats V8" claim (N10) [R15][R17].
- AOT pays in four places: no warm-up on short runs, speed where JIT is forbidden, predictable latency, and no JIT compile CPU (D15 = a) [R15].
- TypeScript types are guarded hints (HINT), never proof. Stage A4 measures what they buy. K-A4H removes them if the gain is small [R12][R17].
- "Faster than V8" is a target in three gated cases only: FV1 (jitless), FV2 (short runs) and FV3 (sound typed numeric class code, A5, by the D17 rule).
- Every AOT stage has a numeric gate and a written rule. The orchestrator applies each rule at a fixed point and logs it ([11-decisions](./11-decisions.md) §2). The owner may override.

IDs used here: JIT mode, AOT-jitless mode, hybrid mode; T0-T3, T1b, baseline-AOT, optimized-AOT; M0-M10, M5c; H2, H3, H5-H8, H11, H13, H14, H16, H22, H27, H29-H37; D3, D6, D11, D15-D19; G2-G4, G9, G11-G15; N5, N10-N13; A0-A5, A5v; FV1-FV3; K-A3, K-A4J, K-A4P, K-A4H, K-A5pre, K-A5, K-A5v-cov, K-A5v-bound, K-A5v; PROVEN, RUNTIME, PROFILED, HINTED; HINT, SOUND, VERIFIED; DEOPT, SLOWPATH, PIN, THROW; GuardFuse, InvalidatingFuse; RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst; FactSnapshot; SP-Unit, SP-Facts, SP-HintPrecision, SP-TypedBound, SP-RegionCoverage, SP-RegionBound, SP-tsrs-win; HINT-SET, TYPED-KERNELS, SHORT-RUN, JITLESS-SET, STARTUP-SET, SIZE-REPORT; RK29-RK44.

Evidence tags: [R02] [R06] [R10] [R11] [R12] [R13] [R14] [R15] [R16] [R17] [R18]. [R17] wins over [R16]. Paths are in the table at the end.

---

## 1. Short answer

The user asked for one engine that is optimized for both JIT and AOT, with a deep AOT optimizer that uses TypeScript types. Goal statement (v2): **one engine, three deployment modes, one optimizer.**

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
- **Where hints can pay (H30 merge rule, §11).** JIT mode: cold sites and fused facts. Hybrid mode: warm-up and fused facts. AOT-jitless mode: every site, because AOT code is the top tier.

| Mode | Tiers | Top tier | v1 status |
|---|---|---|---|
| JIT (default) | T0 -> T1 -> T2 (-> optional T3) | T2/T3; speed targets G2-G4 unchanged from v1 | ships |
| AOT-jitless | T0 + baseline-AOT (A3) + optimized-AOT (A4), built by ep9c | AOT code (iOS, W^X hosts) | ships; baseline-AOT only if K-A3 passes, optimized-AOT only if K-A4J passes |
| hybrid | AOT units are the start tier; hot functions re-tier through instrumented T1 to T2 | T2 | ships only if K-A4P passes |

A1 artifacts (code cache, startup snapshot, bytecode units) serve all three modes. The full modes table is in [02-architecture](./02-architecture.md).
- **Goals:** G11 dual-mode contracts from M0, G12 three modes in v1, G13 AOT startup (= A1 gate), G14 FV1-FV3 gated, G15 size/startup/RSS per mode.
- **Non-goals:** N10 a general "AOT beats V8" claim, N11 TS types without guards in conformance mode, N12 AOT of every function by default, N13 a lean runtime without compiler and interpreter.

**Product claim (D15 = a).** AOT = startup and warm-up, jitless speed, predictable latency, no JIT compile CPU, and possibly smaller total shipped size. The JIT owns general peak. Claim (b), "beat V8 at peak on sound typed numeric class code", is the gated stage A5 (= M10). It names the mechanism that passed FV3 (D17 rule). "Found automatically" may appear only if automatic regions (D17 d) pass FV3, and only for those kernels. Claim (c), "AOT with TS types beats V8 in general", is rejected (N10). "Predictable latency" is claimed only where the SHORT-RUN latency metric shows it ([10-quality](./10-quality.md) §3). The speed ladders below show where measured engines land, and the gate floor of each ep9 tier and code kind.

```
GENERAL JS: Octane, 7 kernels, untyped; score as a fraction of V8 full [R15]. One '#' = 0.04.
Right: ep9 gate FLOORS, converted with Octane-7 V8 full = 25.62x QuickJS [R06]. Expected values can be higher.

 1.041 bun (JSC)                 ##########################
 1.000 V8 full                   #########################
                                                           <- 0.78  T3 (M9, optional): >= 20x QuickJS
 0.553 V8 Maglev cap             ##############            <- 0.55  T2 aim: 14x QuickJS
                                                           <- 0.39  T2 floor (M7): >= 10x QuickJS (G4)
                                                           <- 0.137 T1b floor (M5): >= 3.5x QuickJS (G3)
                                                           <- 0.10-0.13 optimized-AOT floor (A4, K-A4J)
 0.117 Hermes baseline JIT       ###
 0.094 Static Hermes AOT untyped ##
                                                           <- 0.085 baseline-AOT floor (A3, FV1)
                                                                    (expected 1.6-2.8x T0 = 0.083-0.145 [R14])
 0.078 Porffor AOT (6 kernels)   ##
 0.069 Hermes interpreter        ##
 0.053 node --jitless            #                         <- 0.052 T0 floor (M2): >= 1.33x QuickJS (G2)
 0.039 QuickJS                   #

TYPED NUMERIC CODE: nbody; time as a multiple of V8 full, lower is faster [R15]. One '#' = 0.2.

 0.82 C -O2 -ffp-contract=off  ####        nbody ceiling with JS-shaped data (about 1.22x faster)
  --  scriptc typed subset                 about 1.15x faster than V8 peak [R10][R17]
                                           <- FV3 (A5): >= 1.1x faster than V8 full
 1.00 V8 full                  #####
 1.24 bun (JSC)                ######
 1.92 Static Hermes typed AOT  ##########  today's only sound-typed JS AOT
 2.73 V8 Maglev cap            ##############
 3.93 Hermes JIT, typed        ####################
10.9  Static Hermes AOT untyped (cut)      naive untyped AOT = baseline-JIT class
46.7  node --jitless           (cut)
```

## 2. Where AOT wins and where JIT wins

Evidence class: **JS** = measured on a JS engine; **non-JS** = another VM; **vendor** = the vendor's own numbers; **crafted** = a suite written to show an effect.

**AOT wins or ties**

| Case | Number | Comparator | Class | Tags |
|---|---|---|---|---|
| Jitless platform: AOT is the top tier | Baseline-class AOT gives 1.6-2.8x its interpreter (Hermes native 1.6-2.0x, weval 2.77x). Hermes untyped native gives 1.3-2.0x its interpreter on 3 kernels | own interpreter | JS | [R14][R17] |
| Short runs | V8 kernels reach peak in 2-3 ms (1.4-4 ms excess). tsc warm-up is 26-30% of a 1.6 s run | V8 full | JS | [R15] |
| No JIT compile CPU | V8 spends 1.58 s of background compile CPU in a 1.62 s tsc run. Wall time barely changes with spare cores | V8 full | JS | [R15] |
| Typed numeric code vs a mid tier | Static Hermes typed: nbody 1.42x faster than Maglev cap; raytracer tie | V8 Maglev cap | JS | [R15] |
| Typed numeric class code vs V8 peak | scriptc nbody about 1.15x faster than V8 peak; 1.25-1.28x incl. V8 warm-up; same source | V8 full | JS (subset, refuses eval) | [R10][R17] |
| Closed program with a training profile | Graal AWFY time vs C2: JIT 0.92, native-image 1.02, PGO 0.89-0.91 (trained on the same benchmarks) | own JIT, C2 | non-JS | [R11][R17] |
| Predictable first iteration | Leyden profile replay: first iteration -31%. native-image first iteration 0.9-1.4x of steady state; C2 2.07x | own JIT | non-JS | [R11] |
| Megamorphic sites, closed world | Dart AOT/JIT: megamorphic 0.50-0.69; provably mono 0.85 | own JIT | non-JS | [R11] |
| Total shipped size | Dart JIT VM 41.5 MB vs AOT runtime 4.8 MB. ASP.NET NativeAOT size -89% | own JIT runtime | non-JS, vendor | [R11][R17] |
| Bytecode caching (A1, no machine code) | Hermes .hbc 14.9 vs 836 ms (its slow lazy source path); V8 cache 2.0x; bun --bytecode 2.1x; qjsc 10x. Expect 2-10x | own source path | JS | [R10][R17] |

**JIT wins**

| Case | Number | Comparator | Class | Tags |
|---|---|---|---|---|
| Sound-typed AOT today | Static Hermes typed native: nbody 173 vs 90 ms (1.92x), richards 288 vs 91 (3.16x), raytracer 148 vs 96 (1.54x); geomean about 2.1x slower. richards 2.2x slower than Maglev cap | V8 full, Maglev cap | JS | [R15] |
| Typed subset on OO code | scriptc richards about Maglev speed (259-334 vs 263-308 ms) | V8 Maglev cap | JS | [R10][R17] |
| Naive untyped AOT (generic runtime calls) | Static Hermes untyped 10-13x slower than V8, 1.3-2.0x its interpreter, slower than Hermes JIT. Porffor 11-18x slower | V8 full, own JIT | JS | [R15][R17] |
| Untyped AOT with inference | Hopc: many JetStream tests 1-2x slower than V8 6.8 (2018); current V8 likely widens the gap. Weak on FP and call-heavy code; >= 3x less memory. Still far ahead of naive untyped AOT. Its authors call AOT vs JIT "still unanswered" | V8 6.8 (2018) | JS (research) | [R10][R11][R17] |
| General untyped code | Octane fraction of V8 full: Static Hermes native 0.094, Hermes JIT 0.117, Maglev cap 0.553, bun 1.041 | V8 full | JS | [R15][R17] |
| Real app, untyped | Meta tsc native 1.20 s vs node 354 ms (Linux x86-64). Binary 26.3 -> 69.8 MB | node (V8) | vendor | [R15][R17] |
| Untyped JS in a typed-subset compiler | scriptc on untyped JS: 107x slower than V8 | V8 full | JS | [R10][R17] |
| Polymorphic-in-practice sites | Dart AOT/JIT: poly 1.94, Richards 1.28. .NET NativeAOT Dictionary code 2.1-5.3x slower; .NET never instruments R2R code | own JIT + PGO | non-JS | [R11][R17] |
| Replayed profile | Leyden Havlak steady state 20-40% slower (one workload) | own JIT | non-JS | [R11] |
| Per-app payload | Native/bytecode bytes: Sparkplug 6.9x, Maglev 11.6-19x, TurboFan 11.9-28.6x, Static Hermes untyped 10-14x, typed 3.4-4.7x | bytecode | JS | [R14] |

**Not AOT effects.** Hello-world startup is QuickJS 1.99 ms (from source), bun 5.77 ms (JIT) and node 23.5 ms. That is runtime bootstrap and snapshot design. Peak RSS of Hermes JIT (8.8-16.1 MB) is about the same as Static Hermes native (9.8-17.0 MB). ep9 measures both in its own runtime (G15) [R15][R17].

**Ceilings.** C with JS FP rules runs nbody in 74 vs 90 ms (about 1.22x). That is the ceiling on nbody with JS-shaped data only. No OO ceiling exists: bun beats V8 on richards (74 vs 91 ms) and raytracer (77 vs 96 ms) [R15][R17]. Static Hermes's gap is implementation debt, not a ceiling. Its notes list NaN-boxed numbers, no native ints, 5 inline slots, out-of-line typed-array loads, redundant casts and little inlining [R10][R15][R17].

## 3. What "faster than V8" means

| Comparator | Command | Meaning | Used by |
|---|---|---|---|
| V8 full | `node` | peak of a top-tier JIT | FV3; FV2 reported only |
| V8 full, cached | `node` with a warm `NODE_COMPILE_CACHE` | peak JIT plus V8's own code cache | FV2 (wall time; the claim) |
| V8 Maglev cap | `node --max-opt=2` | mid-tier class | T2 reference; A4 context |
| V8 jitless | `node --jitless` | interpreter only | FV1; T0 gate G2 |
| ep9 itself | T0, baseline-AOT, JIT-only with code cache, A4 hinted code, T2 | internal gain | FV1-FV3, K-*, AOT-vs-own-JIT report |

| ID | Case | Stage and gate time | Gate (est.; comparator named) | Evidence | Kill |
|---|---|---|---|---|---|
| FV1 | AOT-jitless where JIT is forbidden | A3; gate at M8 exit, or M5 exit if D16 = yes | AOT-jitless Octane-7 >= 1.6x node --jitless and >= 1.6x ep9 T0 | Baseline-class AOT gives 1.6-2.8x its interpreter (Hermes native 1.6-2.0x, weval 2.77x) [R14]; T0 is gated at V8-jitless parity (M2) | K-A3 |
| FV2 | Short runs (A4 hybrid) | A4; gate by M8 exit | SHORT-RUN (tsc 1-2 s run) wall time <= 0.9x node with a warm compile cache (cold node reported too); time-to-peak >= 15% better than ep9 JIT-only with code cache; steady state within 2%. Depends on G4 (T2 TS workload <= 0.6 s) | Maglev-class tier = V8 full on the TS workload (0.53 vs 0.55 s) [R06]; warm-up is 26-30% of a 1.6 s tsc run; 1.58 s background compile CPU [R15]. node's compile cache alone loads typescript.js 2.0x faster (175.6 vs 87.8 ms), about 4-9% of a 1-2 s run [R10][R17] | K-A4P |
| FV3 | Peak on sound typed code | A5 = M10, optional; needs a D17 build choice ((c) or (d)) and a T3-class backend | >= 1.1x V8 full at peak on >= 2 of 4 TYPED-KERNELS and >= 1.3x ep9 A4 hinted code | scriptc about 1.15x V8 peak on nbody; C with JS FP rules about 1.22x, on nbody only [R15][R17]. Only nbody has supporting data. richards: typed AOT ≈ Maglev (scriptc) or 3.16x slower (Static Hermes). raytracer: Static Hermes 1.54x slower. deltablue: no data [R10][R15] | K-A5pre, K-A5v, K-A5 |
| - | NOT CLAIMED | - | Peak on general, polymorphic, untyped JS. Hello-world startup and RSS as AOT effects | Static Hermes untyped 10-13x slower; Dart poly 1.94x; .NET NativeAOT 2.1-5.3x slower on Dictionary code [R11][R15]; [R17] | N10 |

- **FV1 floor.** FV1 beats V8 jitless. At its floor (0.085 of V8 full), ep9 is still below Static Hermes untyped native (0.094) and Hermes JIT (0.117) [R15].
- **AOT vs ep9's own JIT (report, no gate).** This tests the premise "AOT for even faster performance" directly. At A4, ep9 runs optimized-AOT (jitless build, PROFILED + HINTED) and T2 (JIT mode) on JITLESS-SET, HINT-SET and TYPED-KERNELS. It reports the steady-state ratio and time-to-peak. The report goes into the M8 AOT ship review log, as D15 input. Graal PGO beat its own JIT only with a profile trained on the same benchmarks [R11][R17].

FV3 needs a T3-class backend. Typed AOT competes only with these: int32/range inference, inline array access with hoisted bounds checks, unlimited inline slots, no repeated casts, exact FP rules [R15]. They are T2/T3 optimizer passes, so A5 reuses them.

## 4. Type trust levels

| Level | What goes in | Guard | On failure | Where |
|---|---|---|---|---|
| PROVEN | Facts ep9 proves from JS semantics: never-reassigned module bindings, const, scopes without direct eval or `with`, bitwise results are int32, own range analysis, spec invariants | none | - | every mode |
| HINT (default for TS types) | HINTED entries from .ep9h: repr category, class receiver -> shape + slot candidate, call target, non-null, element type, enum constant, field type | tag test, shape check, null check, elements-kind check hoisted to loop entry | JIT: DEOPT + exit counter. Hybrid: DEOPT -> instrumented re-tier. Jitless: SLOWPATH or DEOPT within budget, then PIN. Never THROW | every mode, only if K-A4H passes |
| SOUND (opt-in, A5) | Per-module typed subset: nominal classes, exact fields, final methods, checked casts, bounds-checked arrays, no Proxy/eval/defineProperty on typed objects | only at module, `any`, .d.ts and JS boundaries | THROW TypeError at the boundary. This changes JS semantics | outside test262 mode; only if the D17 rule picks (c) |

- **VERIFIED (candidate, D17 d).** Facts that the `ep9c` verifier proves per region, under entry checks, hoisted interior checks and fuses. Never THROW; legal in test262 mode. It exists only if the D17 rule picks (d). Until then H30 reserves the provenance value and nothing is built [R18].

Full guard table per mode: [04-execution](./04-execution.md). Budgets: §10 here.

**Demote or drop rules**
- Hints from .d.ts or lib declarations get low confidence or are dropped. They are claims about foreign code: 65% of Preact member accesses resolve to hand-written .d.ts [R13].
- A file with type errors loses all its hints [R13].
- A value that flows from `any`, `as` or `!` loses its hint [R12].
- A structural interface receiver only pre-sizes the IC. It never becomes a shape guard [R13].
- Runtime feedback always wins on a conflict: RUNTIME > PROFILED > HINTED (H30 merge rule, §11). A site with 4 guard failures (est.) stops using its hint, in every mode.

**Why hints, not proof.** Each of 24 known hole classes passes strict tsc/tsrs and lies at runtime. This is a crafted suite, not a rate [R12][R17]. The tsrs hint is wrong at runtime in each of 13 crafted oracle holes [R13]. Real code carries little codegen data: int-typed 1-9% of numeric expressions (TS compiler 37%), class receivers 0.4-61%, 3-42 casts per KLOC [R12]. Porffor trusts types unchecked and prints 17 where JS prints 'a1' [R10].

## 5. Where hints should pay (hypotheses, unmeasured)

| # | Use | Mechanism (always behind a guard or fuse) | Data that suggests a gain | Tags |
|---|---|---|---|---|
| a | Cold-site feedback pre-fill | write HINTED entries into feedback slots before the first run | "insufficient feedback" = 33% of 1,693 tsc deopt events (events, not time) | [R12][R17] |
| b | Shape + slot candidate | shape guard + direct slot load | slot found: typed AWFY 64% vs TS src/compiler 0.2% | [R13] |
| c | Call target + CHA | direct call under a fuse on the prototype chain (H31) | 97% of calls resolved in typed class code | [R13] |
| d | Enum folds | constant under a fuse, because enum objects are mutable | 32% of member accesses in TS src/compiler | [R13] |
| e | Field-type seeds, slot pre-sizing | seed the field-type lattice; pre-size in-object slots | field-type dependency deopts: about 10% of tsc deopt events | [R12][R17] |

- **Per mode (H30 merge rule, §11).** In JIT and hybrid mode, a site with RUNTIME feedback ignores HINTED facts of the same kind. There, hints pay at cold sites through (a) and (b), and at every site through the fused facts (c), (d) and slot pre-sizing (e). In AOT-jitless mode, (a)-(e) apply at every site.
- **What hints cannot fix.** "wrong map" was 49% of tsc deopts; interface types give no layout. "not a Smi" was 0.4%; int32 speculation almost never fails [R12].
- **Ceiling at peak.** Guard removal gave 2.2-4.6% (IISWC'16) and about 8% (IISWC'21 abstract). Sound types in 2015 V8 (StrongScript) gave -0.9% to +22%. Guarded hints are unmeasured [R12][R17]. At warm sites with feedback, only fused hints (c, d) remove guards, so this ceiling is an upper reference.
- **NaN-boxing note.** V8's `x;` class-field cliff is 1184-1428 vs 196-201 ms. JSC (NaN-boxed) shows 237 vs 231 ms [R10][R17]. ep9 uses JSC-style NaN-boxing (H6), so field-type hints buy less than in V8. They help slot pre-sizing and tag-check removal.
- **SP-HintPrecision (A1h).** It reports the `any` share, class-receiver share and slot-found share per HINT-SET group. Early off-ramp: class-receiver share < 5% and slot-found < 5% on >= 2 of the 3 real-app groups (TS compiler, vscode, hono). Then D18 becomes (c) for HINTED consumers before A4 (D18 rule).
  - Typed AWFY is the best case by construction (class receivers 91%, slot found 64% [R13]), so it does not count.
  - Expected from known data: TS compiler is below the line (class receivers 0.4% [R12]; slot found 0.2% in src/compiler [R13]). hono (21.2%) and vscode (59.2%) are above it [R12]. So the off-ramp fires only if SP-HintPrecision finds far lower shares in hono or vscode.

## 6. ep9-typecheck: the tsrs type oracle

```
 tsconfig.json + sources (decoded text)
   -> ep9-typecheck (tools/ep9-typecheck: own [workspace], tsrs crates by path, tsrs Cargo.lock, 1.99.1)
        1 config      tsoptions::get_parsed_command_line_of_config_file
        2 host        new_cached_fs_compiler_host + bundled lib .d.ts
        3 program     ProgramOptions (single_threaded true|false) -> new_program; N checkers
        4 full check  get_diagnostics_of_any_program -> has_type_errors per file
        5 queries     program.get_type_checker_for_file(sf): one checker per file, types never mixed
        6 write       .ep9h through ep9_hints (zero deps); records sorted by (start, end, kind)
   -> app.ep9h -> ep9c build --hints -> ep9_frontend join (01) -> HINTED entries -> ep9_opt (A4)
```

- **Queries (public API) [R13].** get_type_at_location (flow-narrowed); get_symbol_at_location_exported; get_apparent_type; get_non_nullable_type; get_resolved_signature_exported + get_return_type_of_signature (call targets); get_properties_of_type and get_base_types (class layout); is_array_type, get_type_arguments, is_tuple_type_exported; get_constant_value (enums). Classification uses TypeFlags and ObjectFlags only.
- **Not public.** get_iterated_type_or_element_type (for-of element types) and raw expression types. These may cost local patches.
- **Cost [R13].** 10 MB of TS: check 0.46 s, export 1.2 s, 223 MiB. 18 MB with 16 checkers: 0.87 s wall in total. The output is byte-identical to one checker. Clean build 38 s; binary 13.6 MB.

**Rules**
- Hints are optional in every mode. JIT mode needs none. Plain JS uses PROVEN and PROFILED facts only. Unix first; Windows follows SP-tsrs-win.
- Drift budget: <= 1 PM/year and <= 2 local tsrs patches (est.). Measured drift: a 2-line fix against 8-day-old tsrs, 6 compile errors against 15-day-old tsrs [R13]. Upkeep above 1 PM/year trips K-A4H.

**Why a separate binary**
1. tsrs pins unicode-id-start =1.1.2 (Unicode 15.1). Linked with oxc, it makes oxc fail 16 test262 Unicode identifier tests [R02][R13].
2. tsrs reserves a fixed 32 GiB at 0x4001_0000_0000. Its arenas live until exit, and it has about 220 globals and abort paths [R13].
3. tsrs internals churn (about 200 commits a week). The ep9-owned schema H37 absorbs this [R13].
4. tsrs is Apache-2.0 and derived from TypeScript. Its obligations stay inside a build tool [R13].

Join rules: [01-front-end](./01-front-end.md). Build and sync: [08-repo](./08-repo.md).

## 7. Side-table schema (H37, crate ep9_hints)

```
 .ep9h
 +- header      magic | schema version | tsrs commit | fact-schema version | TS flags
 |              (useDefineForClassFields, experimentalDecorators, emitDecoratorMetadata)
 +- file table  path | decoded-text hash | has_type_errors | record range
 +- classes     class id -> decl file:offset, field order, slot count
 +- records     per file, sorted by (start, end, kind), about 20 B each:
 |              repr category | class id | slot candidate | call-target decl | non-null |
 |              enum const | element type | dts/lib | confidence
 +- strings     paths only; no type strings
```

- **No type strings.** They add about 620 MiB at export. The JSON prototype was 195 B per node; the binary target is about 20 B [R13].
- **Join** (owned by [01-front-end](./01-front-end.md)): key (start, end, kind) + decoded-text hash. A mismatch, or has_type_errors = 1, drops the whole file's hints. Drops are counted.
- **Conversion.** BytecodeGen turns joined records into HINTED entries keyed by (function id, feedback slot). It adds function-entry records (parameter and return repr) and class records (field types, slot count). The entry format lives in ep9_facts. Entries go into the CompiledUnit hint section.
- **Versions.** v0 at A1h (by M4 exit); frozen at A4 start. A header mismatch (schema, fact schema or TS flags) means ep9 ignores the file.

## 8. AOT unit format (H35)

```
 ep9 unit, one container: .ep9u blob (mmapped, servers) or .o sections (linked into a signed app, iOS)
 +-------------------------------------------------------------------+
 | header       versions, CPU features, content hash, (signature)    |
 | text (RX)    baseline-AOT and optimized-AOT code; zero relocations|  shared per process
 | rodata       deopt translations, safepoint + pc->bytecode maps,   |
 |              position tables, function table                      |
 | RW template  link table (roots, builtins, atoms, fuses), IC cells,|  copied per isolate
 |              exit + call counters, fuse refs                      |
 | bytecode     every compiled function and inlined callee (H2)      |
 | source       always (Function.prototype.toString, lazy compile)   |
 | hints        optional HINTED section (ep9_facts format)           |
 | unwind/debug unwind tables, perf map                              |
 +-------------------------------------------------------------------+
```

**Header fields.** Magic and container version. Engine build id. Bytecode ABI version (H2). Value-encoding (H6) and object-layout (H8) versions. Barrier version (H11): Leyden rejects AOT code when GC barriers differ [R11]. Frame, IC, deopt-metadata and fact-schema versions (H14, H16, H29). CPU features and target: Leyden also checks CPU features [R11]. TS flags (H3). Content hash and optional signature (D6).

**Rules**
- Zero text relocations. The writer rejects HeapConst. An xtask check enforces this; wasmtime refuses text relocations too [R14].
- IR heap refs (RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst; H30) lower to link-table loads, RW cells or unit constants (H34).
- AOT code reaches the isolate through a reserved isolate register. It reaches its RW data pc-relative.
- Link cells and IC cells are GOT-like. Link cells resolve at load. IC cells fill at first execution. GC rules for them: [03-gc](./03-gc.md).
- ShapeId immediates appear only for snapshot root shapes (H8). Other shape guards compare the object's ShapeId with an RW cell, like Static Hermes {clazz, slot} entries [R14][R17].
- Realm values come through IntrinsicRef: a load from the current realm's intrinsics table by stable index (H13), never a raw pointer. Text is shared per process; RW data is per isolate (H22).
- Source is always included: 80 test262 files test Function.prototype.toString [R14] (H5). The .o form uses static registration.

```
 load unit (.ep9u or .o)                      code cache entry: steps 1-3 only
   +-1 prefix: magic, container version, header size, section table -- fail -----> LOAD ERROR
   +-2 content hash (+ signature if D6 requires) ---------------------- fail -----> LOAD ERROR
   +-3 engine build id, bytecode ABI version ------------------------- mismatch -> SOURCE
   +-4 value-encoding, object-layout, barrier, frame, IC, deopt-metadata,
   |     fact-schema versions; CPU features and target --------------- mismatch -> BYTECODE
   +-5 map text RX, shared per process
   +-6 copy RW template per isolate; resolve link cells (roots, builtins, atoms,
   |     fuses); IC cells start empty; IntrinsicRef reads the current realm
   +-7 register pc->code registry, unwind info, perf map, GC visitor
   +-8 install dispatch slots: aot-baseline / aot-opt
```

| Outcome | Action | Why |
|---|---|---|
| LOAD ERROR | `ep9 run --unit`: clean error, exit code 1, no section used. A statically linked `.o` unit: the same clean startup error, exit code 1. Code cache: drop the entry, compile the source file, write a new entry | A bad prefix hides where the sections are. A failed hash or signature leaves no trusted byte, the source section included. Hermes also rejects a bad magic or bytecode version (`BCProvider.cpp` sanityCheck) |
| SOURCE | Compile the unit's source section (always present, H5) with the header's goal, strict and TS flags -> T0. Drop the bytecode, native code and hints sections | Bytecode of another ABI cannot run. Another engine build can change bytecode meaning ([07-runtime](./07-runtime.md) §3) |
| BYTECODE | Finalize the unit's bytecode -> T0 (-> JIT in a JIT build). Drop native code. On a fact-schema mismatch, drop the hints section too | These versions describe native code only. The bytecode ABI has its own version (H2) |

- Decision H35-fallback (2026-10-20; [H35 S3-S4](../adr/H35-aot-unit-format.md)).
- Every outcome other than a full load is counted by reason and prints one warning line per unit, as Leyden does by default [R11]. `--unit-strict` turns SOURCE and BYTECODE into LOAD ERROR, as Leyden's `-XX:AOTMode=required` does.
- With an exact build-id match, step 4 fails in practice only on CPU features. The per-contract versions stay: they keep the mutation tests exact, and a later ADR can relax step 3 to "same bytecode ABI -> BYTECODE" without a format change.
- The A2 gate needs 100% of header mutations to end in the outcome above. Never a crash.

## 9. What AOT compiles (D19)

```
 ep9 run --profile-out app.ep9prof app.js        training run (JIT build or T0)
   -> .ep9prof: FactSnapshots per function + call counts + source hashes + fact-schema version
 ep9c build --baseline | --opt  --profile app.ep9prof  [--hints [app.ep9h] --tsconfig t]  [--all]
   -> module graph from original sources -> hot set -> FactSnapshot per function
   -> ep9_baseline / ep9_opt -> ep9_masm (AOT finalize) -> ep9_unit writer -> .o | .ep9u
```

| Mode | baseline-AOT | optimized-AOT | Everything else |
|---|---|---|---|
| JIT | none | none | A1 bytecode unit or code cache |
| hybrid (D19 = b) | optional (A3 units also load) | PGO hot set (A4) | bytecode in the unit; T0 -> JIT |
| AOT-jitless (D19 = c) | all reachable functions (`--all`: every function) | PGO hot set, if K-A4J passes | bytecode in the unit; T0 |

- ep9c never compiles eval or `new Function` code ahead of time. That code compiles at runtime in every mode (H5).
- A function whose source hash differs from the profile ignores the profile. It gets code without PROFILED facts.
- Runtime feedback overrides PROFILED facts (H33). Leyden replay gave a 31% faster first iteration but a 20-40% slower steady state [R11].
- The hot-set threshold is set in A3 (est.). ART profiles about 20% of methods (2-60%). Full AOT failed on Android 6, where large apps took minutes to install [R11][R14].
- The jitless default (c) departs from [R14], which advised a hot set even for baseline-AOT. Without a JIT, cold code would run in T0, so [R16] advises (c) for jitless. The A3 size gate bounds it.
- Native code for all TS-workload functions is about 2.5 MB at baseline density and 4.4 MB at optimized density, before metadata [R14].

## 10. Jitless guard policy: no deopt loops

With no JIT, nothing re-profiles or recompiles, so a deopt can repeat forever. ART emits no deopt in AOT code for this reason ("forever deoptimizations") [R11][R14].

```
 AOT-jitless: a PROFILED or HINTED guard in optimized-AOT code fails
      |
      v
 site exit counter (RW) += 1 --- counter < 4 (est.) ---> DEOPT: rebuild a T0 frame at the bytecode offset
      |                                                    function exit counter += 1
      | counter >= 4 (est.)                                      |
      v                                                          | function exits >= 16 (est.)
 SLOWPATH: generic IC call inside the AOT code                   v
 (the site ignores its fact from now on; no deopt)     PIN: dispatch slot -> aot-baseline
                                                       (-> interp if the function has no baseline-AOT)
```

- Never THROW on a hint failure. A site without a SLOWPATH variant keeps deopting until the function is pinned. SLOWPATH variants cost code size (SIZE-REPORT).
- Invariants use a GuardFuse (load + compare, then SLOWPATH) or an InvalidatingFuse. An InvalidatingFuse lazily deopts by return-address swap and swaps the dispatch slot. It patches no code.
- Budgets (4 per site, 16 per function) are owned here and tuned in A4. Tests: hint chaos and stale profiles ([10-quality](./10-quality.md)).

## 11. Hybrid mode feedback rule (H33)

Optimized AOT code does not produce feedback for free. .NET never instruments R2R code; after 30 calls it re-JITs the method as Tier1Instrumented, then Tier1 with PGO [R14][R17].

| Code kind | Writes | Hybrid re-tier path |
|---|---|---|
| T0, T1 | full FeedbackVector | T0 -> T1 -> T2 by v1 thresholds |
| baseline-AOT | full feedback schema through its RW IC cells (H16) | to T2 when hot |
| optimized-AOT | call and exit counters only | hot -> instrumented T1 -> T2 |

```
 aot-opt --calls >= 1,000 (est.)--> T1 instrumented --T2 threshold--> T2 (RUNTIME facts)
    |                                     ^
    +-- guard fails -> DEOPT -> T0 frame -+   (threshold owned by 04, tuned in A4)
```

- **H30 merge rule** (in the H30 ADR before M1 exit). Precedence RUNTIME > PROFILED > HINTED applies per fact kind. On a conflict, RUNTIME wins.
  - A slot with RUNTIME facts ignores PROFILED and HINTED facts of the same kind (shape, call target, representation).
  - HINTED facts that feedback cannot give stay beside RUNTIME facts: CHA and enum constants under a fuse (H31), non-null, slot pre-size.
  - The JIT graph builder reads them from the CompiledUnit hint section ([04-execution](./04-execution.md) §6).
- Costs to budget: instrumented T1 code for hot AOT functions, compile CPU for re-tiered code, and slower code while instrumented. K-A4P checks the FV2 numbers and test262 equality; on failure, v1 ships no hybrid mode.

## 12. Stages A0-A5

```
 CORE: M0 ---- M1 ---- M2 ==gate==> M5 ---------- M7 ---------- M8 ---- [M9]
 AOT : A0(1) - A0(2) -+- A1h (due M4 exit)
                      +- A2 (due M5 start) - A3 (with M5) - A4 (with M7) - gates at M8 - [A5 = M10]
 RT  :          M4 (+A1) ------------- M6
 Rule: no JIT tier code before M2. A2 may start after M1 exit behind `aot`, with no tier that runs JS.
```

| Stage | Inside | Lane | Scope (deliverables) | Gate (est.) | Kill | PM (est.) |
|---|---|---|---|---|---|---|
| A0 Foundations | M0 (part 1), M1 (part 2) | AOT senior + CORE leads | ADRs H2, H3 fields, H5, H7, H8, H13 rules, H14, H27, H35 spec v0, H36 (M0); H16 frozen as FactSnapshot schema, H29-H34 frozen (M1); container spec v0; determinism CI; SP-Unit; SP-Facts | SP-Unit green on 4 targets. SP-Facts: Spike C IR uses the fact lattice + ref nodes and PI deopt metadata; one kernel from a PROFILED FactSnapshot file gives identical output at >= 90% of live-feedback speed. Identical bytecode hashes across two builds per target | none (contract) | 1-2 |
| A1h Hint oracle v0 | after M1 exit, due M4 exit | AOT | tools/ep9-typecheck builds (unix); ep9_hints schema v0; join in ep9_frontend (no codegen use); SP-HintPrecision report on HINT-SET; SP-RegionCoverage (K-A5v-cov) | join >= 99.5% of core node kinds | early D18 off-ramp; K-A5v-cov | 1-3 |
| A1 Bytecode AOT | M4 | CORE (serializer, snapshot 2-4 PM, already in v1 M4) + AOT (packaging) | code cache, startup snapshot, `ep9c build --bytecode` | typescript.js + 1 call loads >= 2x faster from a unit than the ep9 cold source path; snapshot start >= 2x; warm tsc with cache >= 1.5x; 100% invalidation tests | none (the runtime needs it) | 1-2 |
| A2 Unit machinery | after M1 exit, due M5 start | AOT (+ CORE JIT lead review) | ep9_masm relocation kinds; ep9_unit writer/loader (.o + .ep9u); link table; RW cells; registry entries; unwind on 4 targets; version fallback; optional embedded stub corpus | a unit with stub calls loads as blob and .o on 4 targets; zero text relocations; 100% of header mutations end in their H35 S3 outcome; exceptions and Error.stack through AOT frames; GC-stress clean with AOT frames | none | 4-8 |
| A3 Baseline-AOT + AOT-jitless mode | built with/after M5 (T1); gate at M8 exit, or M5 exit if D16 = yes | AOT | `ep9c build --baseline`, PGO hot-set selection, `--profile-out`, jitless load path | FV1: JITLESS-SET >= 1.6x node --jitless and >= 1.6x T0; test262 equal; TS workload <= 1.8 s; unit text <= 8x the bytecode of compiled functions | K-A3 | 6-12 (+2-4 iOS if D16 = yes) |
| A4 Optimized-AOT, hybrid, hints | with M7; gates by M8 exit | AOT + CORE T2 owners | T2 AOT emit, guard variants (H32), PROFILED and HINTED inputs, hybrid re-tier (H33), `ep9c build --opt` | A4-J (K-A4J rule), FV2/A4-P, A4-H (K-A4H rule) | K-A4J, K-A4P, K-A4H | 16-32 |
| A5 Sound typed code (= M10, optional) | after M7 (+ M9 backend for FV3) | AOT | by the D17 rule: (d) automatic VERIFIED regions in `ep9c` AOT units (finder + verifier, fuses, region code + generic sibling), or (c) an opt-in sound subset (`ep9c build --sound`) | FV3; for (d) also K-A5v | K-A5pre, K-A5v-cov, K-A5v-bound, K-A5v, K-A5 | (d) 16-33 (10-21 own); (c) 12-24 prototype |

**v1 (M8) ships:** JIT mode and A1 artifacts. AOT-jitless mode ships with baseline-AOT if K-A3 passes (else M5c if ready, else T0 only). Optimized-AOT ships if K-A4J passes, hybrid if K-A4P passes, HINTED facts if K-A4H passes. Milestone detail: [09-milestones](./09-milestones.md).

## 13. Kill criteria

| ID | Checked | Rule (est.) | If it trips | Kept | Data to |
|---|---|---|---|---|---|
| K-A3 | Pre-check when baseline-AOT first runs JITLESS-SET (target M5 exit, whatever D16 says); final at the A3 gate | baseline-AOT < 1.3x T0 on JITLESS-SET | Stop baseline-AOT emit. Build M5c, a generated interpreter embedded at engine build time, as the jitless top tier (3-6 PM est., AOT lane, in the M7 window). PIN and K-A4J then use M5c or T0 as the base. FV1 is not claimed if < 1.6x node --jitless | T0, A1, unit loader | D16, D11 (pre-check; A3 gate) |
| K-A4J | M8 exit | optimized-AOT (jitless, PGO) < 1.5x baseline-AOT on JITLESS-SET | Jitless ships baseline-AOT only | baseline-AOT | D15, D16 (M8 AOT ship review) |
| K-A4P | M8 exit | hybrid misses the FV2 time-to-peak or steady-state numbers, or test262 differs | No hybrid in v1 | A1 + jitless AOT | D15 (M8 AOT ship review) |
| K-A4H | M8 exit | Keep HINTED consumers only if (>= 3% geomean wall-time gain on HINT-SET in at least one mode OR >= 10% better time-to-peak on SHORT-RUN) AND no HINT-SET workload > 1% slower. Also kill if ep9-typecheck upkeep > 1 PM/year | Remove HINTED consumers | `ep9 check` | D18 (M8 AOT ship review) |
| K-A4H (early off-ramp) | A1h (SP-HintPrecision) | class-receiver share < 5% and slot-found < 5% on >= 2 of the 3 real-app HINT-SET groups (§5) | D18 = (c) for HINTED consumers (D18 rule) | A1h oracle (ep9-typecheck stays only if K-A5v-cov passed) | D18 |
| K-A5v-cov | A1h (SP-RegionCoverage) | hot-time share in finder-v1 regions < 15% on >= 2 of the 3 real-app HINT-SET groups (TS compiler, vscode, hono); the share is the lower of a V8-default and a `--jitless` profile | No real-app A5v. D17 (d) stays possible for kernels only (D17 rule step 2) | the finder report | D17 rule (M7 exit) |
| K-A5pre | M7 (SP-TypedBound) | T2 with guards removed (test-only trusted mode) on TYPED-KERNELS < 1.15x normal T2 | No (c) (D17 rule step 3) | - | D17 rule (M7 exit) |
| K-A5v-bound | M7 (SP-RegionBound, jitless) | region code < 1.1x A4 code inside regions on TYPED-KERNELS | No A5v | - | D17 rule (M7 exit) |
| K-A5v | after the A5v prototype | Real-app part: < 3% geomean end-to-end gain on the 3 real-app HINT-SET groups (typed AWFY excluded), AOT-jitless vs A4 optimized-AOT. Kernel part: FV3 not met. Stop only if both parts fail; a part that fails loses only its claim | Stop A5v, or drop the failed claim | A4 code | decisions log |
| K-A5 | after the A5 (c) prototype | FV3 not met | Stop A5 (c) | - | decisions log |

- **K-A5v-cov today (provisional).** R18 data trips it: tsc 6.7-8.8%, hono 0.2-6.2%; the vscode diff engine passes with 23.9-43.4% [R18]. So no A5v work and no A5v contract content before A1h.
- **A5v safety stop.** Any open type-confusion or abort bug that fuse, inspector or entry-check chaos finds in region code turns region code off (dispatch to the generic sibling) until it is fixed.
- **A5v dependency.** If K-A4J trips at M8 exit, A5v loses its T2 AOT emit path, so D17 (d) stops.

- **Who decides.** The AOT lead agent measures. The orchestrator applies the rule at the points in [11-decisions](./11-decisions.md) §2 and logs the data and the result. Losing parts leave the v1 build by their rule. The owner may override any result.
- **Why every stage has one.** Naive AOT can lose to tiers ep9 already has. Hermes untyped native was slower than Hermes' own baseline JIT on every kernel [R14][R17].

## 14. Effort

| Stage | PM (est.) | Breakdown |
|---|---|---|
| A0 | 1-2 | ADRs, determinism CI, container spec v0, SP-Unit (+0.25), SP-Facts (+0.25-0.5) |
| A1h | 1-3 | oracle tool, ep9_hints v0, join, SP-HintPrecision, SP-RegionCoverage (≤ 1) |
| A1 | 1-2 | AOT packaging only; CORE serializer and snapshot (2-4) are already in v1 M4 |
| A2 | 4-8 | relocation kinds, writer/loader, unwind on 4 targets, fallback |
| A3 | 6-12 | baseline-AOT emit, hot set, jitless path (+2-4 iOS if D16 = yes) |
| A4 | 16-32 | driver + PGO 3-6, T2 AOT emit 6-12, re-tier 2-4, hints 3-6, tests 2-4 |
| A5 | (d) 16-33 (10-21 own); (c) 12-24 | by the D17 rule (M10, optional); prototype |
| M5c (only if K-A3 trips) | 3-6 | generated interpreter from the T1 code generator, embedded through A2. Stopped A3 emit work partly offsets it (est.) |
| **AOT total M0-M8** | **29-59** | A0-A4; A1h-A4 = 28-57 |

- **Staffing.** The AOT plan role runs from M0; under D3 an agent fills it. In M0-M1 it also works on CORE contract items (H2, H14, Spike B). The AOT lane fits inside CORE's window. Total M0-M8 is about 285-524 PM (v1 256-465 + AOT 29-59, est.), a size of work; calendar months do not apply (D3). If A4 trails M7, v1 waits for the A4 gates (D15 timing t1).
- **Calibration** [R14]. Static Hermes, Meta's AOT + JIT engine, still has no speculative tier after years of work. Dart keeps about 276 AOT branches in about 237k lines of compiler code. HotSpot removed jaotc (JEP 295) in JDK 17 (JEP 410): "little use", and its upkeep was "significant".
- **Rule.** The AOT lane never gates M2, M5 or M7 (RK22, RK43). It gates the M0 and M1 exits only through the A0 spikes SP-Unit and SP-Facts, each time-boxed ([09-milestones](./09-milestones.md) §4). The CORE T2 owners join A4 for T2 AOT emit.

## 15. Size

| Item | Data | ep9 rule |
|---|---|---|
| Per-app payload | JIT mode ships bytecode. Baseline and mid-tier native code is 6-19x the bytecode bytes; TurboFan with inlining reaches 28.6x [R14] | JIT mode gives the smallest payload |
| Total shipped size | Dart JIT VM 41.5 MB vs AOT runtime 4.8 MB; ASP.NET NativeAOT -89% [R11][R17] | the jitless build drops runtime codegen and may be smaller in total |
| Compiler cost | the compiler is 2.4 MB of Hermes' 6.0 MB runtime [R14] | kept in every build (H5, N13) |
| Whole-program AOT | Static Hermes tsc 26.3 -> 69.8 MB [R15] | PGO hot set for hybrid and optimized-AOT (D19). Jitless baseline-AOT covers reachable functions, bounded by the A3 gate: unit text <= 8x bytecode |

| Binary | Links | Loads |
|---|---|---|
| ep9 (JIT build) | T0, compiler, T1, T2, [T3] (`jit`), unit loader, deoptimizer | source, A1 units, hybrid units |
| ep9 (jitless build / iOS library) | T0, compiler, unit loader, deoptimizer; no `jit` | source, A1 units, AOT-jitless units |
| ep9c | the engine + `aot` emitters (ep9_aot, ep9_baseline, ep9_opt) | build tool; not shipped |

G15 reports these per mode from A1 (SIZE-REPORT, STARTUP-SET). There is no size target before the A3 review.
- binary size per build config;
- unit size per code kind: bytecode-only, baseline-AOT, optimized-AOT;
- native/bytecode ratio;
- startup and RSS, measured in ep9's own runtime.

## 16. AOT risks (RK29-RK44)

Effects, early signals and full columns: [10-quality](./10-quality.md).

| ID | Risk | Main mitigation | Tags |
|---|---|---|---|
| RK29 | Overselling AOT | D15 = a; FV gates; N10 | [R15][R17] |
| RK30 | Dual-mode test-matrix cost | mode x code-kind matrix nightly from A3/A4; one optimizer | [R14][R16] |
| RK31 | Naive AOT slower than existing tiers | K-A3, K-A4J | [R15][R17] |
| RK32 | Unsound hint without a guard -> type confusion | guards mandatory; hint chaos fuzzing | [R12][R13] |
| RK33 | Jitless deopt loops | site and function budgets; PIN | [R11] |
| RK34 | Code-size blowup | PGO hot set (hybrid, optimized-AOT); A3 size gate (jitless baseline-AOT); SIZE-REPORT | [R14][R16] |
| RK35 | Stale or mismatched units | header versions; fallback by field: bytecode, source section or clean load error (H35 S3) | [R11][R14] |
| RK36 | Unit loading security | content hash, optional signature, loader fuzzing, trusted-code model (D6) | [R14] |
| RK37 | Training-profile lock-in | runtime feedback overrides PROFILED | [R11] |
| RK38 | tsrs oracle churn | ep9-owned schema; drift budget | [R13] |
| RK39 | Span join breaks | TS lowering in BytecodeGen, text hash, no hints for bundles, BOM rule | [R13][R17] |
| RK40 | Sound mode vs test262 | D17 (c): opt-in per module, outside conformance mode. D17 (d) never THROWs and stays legal in test262 mode | [R10][R12][R18] |
| RK41 | FMA or libm differences | H36: no FMA, one Rust Math (`ep9_math`, correctly rounded), `clippy.toml` + `xtask fp-check`, FP differential tests | [R10][R15] |
| RK42 | A4/A5 value depends on T2/T3 quality | SP-TypedBound; A5 after M9 work | [R15][R17] |
| RK43 | AOT lane takes review capacity from CORE, or CORE delays A4 | AOT never gates M2/M5/M7; the orchestrator gives CORE reviews first (D3); v1 waits for the A4 gates (D15 t1) | - |
| RK44 | Hybrid feedback cost | H33 rule; K-A4P | [R17] |

## Changes vs plan v1

- New file. v1 had no AOT section. Jitless was only a build (G9). Code cache and snapshot were runtime items (v1 §9.1).
- The premise "AOT for even faster performance" becomes the gated claim D15 = a. Claim (b) is stage A5; claim (c) is rejected (N10).
- New: stages A0-A5, cases FV1-FV3, kill criteria K-*, trust levels PROVEN/HINT/SOUND, ep9-typecheck, the .ep9h schema (H37), the unit format (H35), compile scope (D19), the jitless budgets and the hybrid feedback rule (H33). N5 refined: no LLVM in JIT or AOT; Cranelift only for an optional T3 shared by JIT and AOT emit.
- Unchanged from v1: tier targets G2-G4, the T0 parity gate, and "no JIT tier code before M2".
- Decisions 2026-10-20 ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)): unit mismatch outcome by field (H35); A5 by the D17 rule with candidate (d) VERIFIED regions [R18]; kill rules K-A5v-cov, K-A5v-bound and K-A5v; K-A5 keeps only its FV3 clause; claim (b) worded without a mechanism; the orchestrator applies the kill rules.

## Evidence tags

| Tag | Path |
|---|---|
| [R02] | [../research/02-tsrs-conformance.md](../research/02-tsrs-conformance.md) |
| [R06] | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R10] | [../research/10-js-aot-precedents.md](../research/10-js-aot-precedents.md) |
| [R11] | [../research/11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R12] | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R13] | [../research/13-tsrs-type-oracle.md](../research/13-tsrs-type-oracle.md) |
| [R14] | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R15] | [../research/15-aot-perf-reality.md](../research/15-aot-perf-reality.md) |
| [R16] | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R17] | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) |
| [R18] | [../research/18-sound-regions.md](../research/18-sound-regions.md) |
