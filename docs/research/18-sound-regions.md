# Automatic sound regions: can ep9 find and optimize the sound parts of normal TS/JS code, with no opt-in?

Date 2026-10-20. Inputs: five studies of this round (precedents, semantics, gain, finder, hot) and one adversarial review. The review findings are applied in this version. The machine was shared (load average 8-37), so every timing is noisy. No ep9 code exists yet, so every ep9 cost is an estimate (est.).
Scratch alias: `SR` = the research folder `work/sound-regions/` in the session scratchpad. These paths are session-local (see README). The semantics and hot studies did not save a report file. This doc inlines the data it uses from them. Plan paths are relative to the repo root. Copies of the scripts, probes and cited results are in [artifacts/sound-regions/](artifacts/sound-regions/README.md) (its README maps the `SR/` folders to the copies and lists what was not copied).

## Summary
The user's idea: do not ask users to write a strict TS subset. Let ep9c find the parts of normal code that meet the restrictions, and compile them with sound facts. The idea is sound in full ECMAScript if every fact in a region has one of three justifications: a static proof (S), a check (B), or a fuse (F), and if the fuse and effect-point lists are complete. On failure, the region falls back to generic code for the same bytecode. It never throws. So the regions stay legal in test262 mode, and the semantic change behind D17 is not needed. TS types can only propose candidates; a verifier that cannot read types proves each fact again. The weak point is coverage, not soundness. Functions that pass strict rules (typed bindings only) hold 7-8% of tsc CPU time, 0-6% of hono, 0% of zod and 6-11% of date-fns. Only 0-6.2% of member accesses sit inside such a function and on a qualifying receiver. The vscode diff engine (24-39%) is the exception among real apps. ep9's own typed AWFY port reaches 64-99%, after four relaxations. The gain is small in JIT mode and small to moderate in AOT-jitless mode. SpiderMonkey shipped a related run-time design (Type Inference, with guards on about 41% of accesses) in Firefox 9-82. Warp removed it: faster on Speedometer and real pages, slower on Octane and Kraken.

**Direct answer**
1. Feasible: yes. No opt-in, no source edits, no THROW, test262-legal. Each fact needs S, B or F, or ep9 drops it. It is sound only if the fuse and effect-point lists are complete; this round found four gaps (Finding 1) [medium].
2. Coverage in real code is small: 0-11% of CPU time in tsc, hono, zod and date-fns; 24-39% in the vscode diff engine. Only 0-6.2% of member accesses are inside a region and on a qualifying receiver [high].
3. Gain, end to end (est.): JIT mode +0-2%. AOT-jitless vs A4: tsc, hono, date-fns +0-0.8%; vscode-diff +0.5-10%; kernels up to +16-27% with inner guards [low].
4. Precedent: SpiderMonkey TI did a related thing at run time and was removed; Warp won on real pages and lost on Octane and Kraken [high].
5. Cost: about 16-33 PM (est.), a new offline analysis pass, 8 fuse kinds, a builtin effect model, a second code copy per region, and a new type-confusion surface.
6. Advice: keep D17 = Defer and add the idea as option (d). Today's data limits it to kernels. The only plan change now is a <= 1 PM coverage pre-check (SP-RegionCoverage).

## What this means for ep9
- A new trust level, VERIFIED, would sit between PROVEN and HINT. It adds no new tier. It adds a new offline analysis pass in ep9c (SSA, ranges, a construction model, escape analysis). ep9_opt builds region code from the same bytecode, with fewer guards.
- Nothing changes before M1a. H17 stays as it is unless Spike C or SP-Facts measures a gain. The verifier and region codegen need T2 passes and the A4 emit path (after M7).
- N11 stays as it is. Types never become facts, so the plan keeps its "TS types are hints" rule.
- A5's current kill rule (">= 50% of one module compiles without edits") is a weak proxy. 18 vscode modules and TS `scanner.ts` (82.3%) pass it, yet hot-time coverage stays low. Use hot-time share and end-to-end gain instead.
- Found on the way, not part of A5v: a debugger session can rewrite strict `let` and `const` locals (B28, B28c). So PROVEN facts on locals need an H24 rule. This is a separate plan item (§6.0).

## 1. Mechanism mapped onto ep9

```
BUILD TIME: ep9c -> ep9_aot                                    RUN TIME: ep9_vm + unit code
.ep9h (ep9-typecheck, optional; needs D18 != c) -+
.ep9prof (training run, optional) ---------------+--> FINDER: candidate functions, entry shapes,
bytecode + ESM module graph ---------------------+     ranges, call targets (types only steer here)
                                              |
bytecode + module graph + H38 builtin  -->  VERIFIER (new offline pass; no access to ep9_hints; H37 firewall)
effect model (new)                          each fact: S | B | F, else drop the fact
                                              |
            +---------------------------------+---------------------------------+
            v                                 v                                 v
     S: VERIFIED facts (H30),       B: checks in region table           F: fuse deps F1-F8 (H31
     keyed by region version        (H35 unit section)                  registry, ep9_vm), armed at install
            +------------> ep9_opt graph builder (H30) <----------------+
                           region code  +  generic sibling (baseline-AOT | T0 | JIT tiers in hybrid)

call -> dispatch slot -> entry check (B)       fail -> generic sibling from the start (SLOWPATH); jitless counter -> PIN
                          pass -> region code
                                  hoisted interior check (B; loop entry, resume)   fail -> DEOPT with frame state
                                  aborting check (bounds, after a proof)           fail -> engine bug: abort
                                  effect point (call, accessor, yield, interrupt poll ...) keeps lazy-deopt metadata (H29)
fuse pop (prototype patch, defineProperty, setPrototypeOf, buffer shrink, inspector connect, eval of new code ...)
          -> InvalidatingFuse -> deoptimizer (ep9_vm) lazily deopts region frames + swaps the dispatch slot
Never THROW -> a conforming result (stack limits may differ, as between T0 and T2 today) -> legal in test262 mode
```

| Rule | A region meets the restrictions only if ... | Why |
|---|---|---|
| R1 | It is strict code with no direct eval or `with` in any enclosing scope | Sloppy `arguments` aliases parameters (SR/semantics/probes.log:11, B10) |
| R2 | Each fact has S, B or F. A TS type never justifies a fact | 24 hole classes pass strict tsc and lie [R12] |
| R3 | Each check is one of the three kinds in the check table below | Values from outside can only be known by a check |
| R4 | F dependencies are armed at install. A mismatch with the build record disables the region | pixijs patches prototypes at init (pixijs src/math-extras/init.ts:7-9: `Object.assign(Point.prototype, ...)`) |
| R5 | At each effect point, heap facts drop, except for non-escaping objects and fused facts. Effect points include calls, accessors, ToPrimitive, `yield`, and every interrupt poll (loop back-edge, function entry). Each interrupt poll is a lazy-deopt point that re-reads F7 | `a + evil` re-targets `v.get` mid-expression (probes.log:8, B07). A worker inspector session's `Runtime.evaluate` retyped `box.v` inside a call-free loop (new probe, below) |
| R6 | The region never pops a fuse it depends on | No self-invalidation |
| R7 | JS-visible results and exceptions equal the generic sibling | test262 mode. Some throws are JS semantics to keep: a field define on a non-extensible object throws (probes.log:13, B12) |
| R8 | `yield`, `await`, `using` and class definitions are exits; on resume, the B checks run again as hoisted interior checks | `box.v` changed across `yield` (probes.log:12, B11); `using` ran a hidden dispose call at block end that changed `box.v` (probes.log:23, B22) |

**Check kinds (R3).** Loop entries and resume points are inside the region. A failure there cannot restart the generic sibling from the start. It needs a mid-function deopt with frame state.

| Kind | Where | On failure | Counted in SP-RegionBound |
|---|---|---|---|
| Entry check | Region entry (call boundary) | Run the generic sibling from the start (SLOWPATH) | yes |
| Hoisted interior check | Loop entry; resume point after `yield`/`await` | DEOPT with frame state to the generic sibling at the same bytecode | yes |
| Aborting check | Bounds and other facts that a proof already covers | Fires only on an engine bug: abort (V8 style, Finding 12) | yes |

**Facts and their breakers.** Probe IDs refer to SR/semantics/probes.log (node 24.22, 30 lines, B01-B28c).

| Fact kind | Justification mix | Main breakers (probe IDs) |
|---|---|---|
| Fixed layout (exact shape) | S if the object never escapes; else B (exact-shape check) + F3 | defineProperty, delete, freeze (B21); setPrototypeOf on the class (B03); base-ctor return override, also with Proxy (B02); Proxy in the chain under [[Set]] field lowering (B04a, B04b); Reflect.construct with a foreign newTarget (B01); exception mid-construction (B13); cross-realm (B18); structuredClone (B14) |
| Field representation | F4 (field type per descriptor, V8 style); S only if all stores are visible | any store anywhere; call/apply with a foreign `this` (B06); inspector (B25) |
| Call target | B (receiver shape) + F1 (per-key prototype conditions) + F2 | `C.prototype.m = f` (B05); subclass from outside (B17); super.m swap (B19); `Symbol.hasInstance` (B24); builtin patch (B16); prototype pollution (B23); decorators |
| Array kind + in-bounds | S for local arrays; else hoisted interior check (kind + length at loop entry) + F5 + F6 | holes read through the prototype, `length` writes (B08); species (B09); ArrayBuffer transfer, 8 -> 0, and resize shrink, 8 -> 1 (B15) |
| Element shapes (`bodies[i]` is a `Body`) | S only if all stores are visible in a closed module; else a check at each load. Elements kinds record Smi, Double or Object, not the shape of each element. TI-style heap type sets are rejected (§3) | any store of a foreign object into the array |
| int32 range | S (exact JS numerics) + B (tag at entry) + F7 | overflow, -0, debugger writes (B28) |
| No-alias / no-escape | S (module-scope escape analysis; module-private bindings are closed to indirect eval and `new Function`, B27) + F7 | inspector (B25); `Error.prepareStackTrace` leaks receivers of sloppy frames only (B20). FinalizationRegistry callbacks run only between jobs, not inside a synchronous region (B26) |
| Locals (PROVEN today, all tiers) | S + F7 | debugger rewrites strict `let` and `const` (B28, B28c); sloppy `arguments` aliasing (B10, excluded by R1) |

**Fuses F1-F8 (named here; H31 rows).**

| ID | Fuse | Pops on | Precedent |
|---|---|---|---|
| F1 | Per-key prototype conditions (presence, absence, AbsenceOfSetEffect), also on builtin objects | write, define or delete of that key on a watched object | JSC PropertyCondition.h:38-47 |
| F2 | HasPrototype (prototype identity of a shape) | setPrototypeOf on a watched object | JSC PropertyCondition.h:47 |
| F3 | Shape stability (no new transitions from a shape) | a transition away from a stable shape | V8 DependOnStableMap (compilation-dependencies.h:49) |
| F4 | Field type per descriptor (optional, RK49) | a store of another type | V8 field representation and field type dependencies (compilation-dependencies.h:185-193) |
| F5 | Array protectors: no elements on Array/Object prototypes; species | an indexed store on a prototype; a species change | V8 NoElements and ArraySpecies protectors (protectors.h:25,30) |
| F6 | ArrayBuffer detach and resizable-buffer shrink. Option: re-read the length after every effect point for resizable or length-tracking views instead | detach, transfer, resize to a smaller length | V8 ArrayBufferDetaching protector, detach only (protectors.h:19) |
| F7 | Inspector session (pops when a session connects, not when a CDP domain is enabled) | `connect` or `connectToMainThread` | ep9 H24 "deopt all on attach" (docs/plan/02-architecture.md:181) |
| F8 | Closed world for a bundle (optional) | eval or `new Function` of new code; a dynamic import outside the bundle | HHVM RepoAuthoritative (§3) |

EVIDENCE: SR/semantics/probes.log:1-30 (sources SR/semantics/probes.mjs, weak-probe.mjs, scope-probe.mjs, inspector-probe.mjs, debugger-probe.mjs, debugger-probe-const.mjs; engine262 @267d1d4). New interrupt probe: SR/refute/interrupt-probe2.mjs. A worker session called `connectToMainThread` and then `Runtime.evaluate`, with no `Runtime.enable`, no `Debugger.enable` and no `debugger` statement. The main thread's call-free `for` loop saw `box.v` change from number to string at iteration 3757 (SR/refute/probes.log:3), and at 3944, 2635 and 2822 in a rerun (SR/revise/probe2-rerun.log). The plan already shares interrupt polls with safepoints (docs/plan/02-architecture.md:180, H23). Fuse precedents: V8 compilation-dependencies.h:49,106,110,185-193 (v8 main @fb88ee4e4f), V8 protectors.h:19,25,30, JSC PropertyCondition.h:38-47 (WebKit @a3c8dbf35b), SpiderMonkey RealmFuses.h (copies in SR/semantics/dl/).

## 2. Measured coverage

Static share = finder v0 rules (strict subset rules r1-r7, applied to unedited code). Hot share = CPU samples whose innermost frame is a qualifying (Q) function, plus builtins it calls ("self+bi"), node 24.22 `--cpu-prof`. This is time in functions with typed bindings, not time in proven sound regions (see Approximations). Two hot columns: V8 default / inlining off.

| Workload (repo) | Classes Q/total | Functions Q | Body bytes Q (v0 / v3) | Dynamic member access on Q receiver | ... and inside a Q function | Hot time Q-v0 | Hot time Q-v3 | Modules >= 50% Q (K-A5 view) |
|---|---|---|---|---|---|---|---|---|
| tsc (TS src/compiler) | 7/9 | 14.6% | 6.2% / 6.5% | 0.4% | 0.1% | 6.7 / 8.4% | 6.9 / 8.8% | 2/61 (scanner.ts 82.3%) |
| vscode base+editor (diff engine run) | 336/761 | 23.0% | 14.7% / 17.9% | 16.0% | 6.2% | 23.9 / 39.1% | 34.0 / 43.4% | 18/199 |
| hono (fetch; routers) | 8/48 | 22.0% | 9.9% / 10.1% | 0.9% | 0.1% | fetch 0.2 / 0.2%; routers 6.2 / 6.0% | same | 3/41 |
| zod v4 (parse) | 5/12 | 9.4% | 4.0% / 4.3% | 5.5% | 1.4% | 0 / 0% | 0 / 0% | 0/15 |
| date-fns (format + parse) | 3/35 | 13.7% | 4.5% / 5.6% | 1.0% | 0.2% | 5.9 / 11.0% | 6.1 / 11.2% | 0/5 |
| pixijs | 40/306 | 11.2% | 4.8% / 5.3% | 2.4% | 0.5% | not measured | not measured | 3/164 |
| preact (JS) | 0 ES classes | 9.3% | 1.7% / 1.7% | 0% | 0% | not measured | not measured | 0/3 |
| AWFY TS port, ep9's own [R13] (Richards / DeltaBlue / NBody) | 10/30 (v3: 29) | 18.6% (v3: 79%) | 15.2% / 81.6% | 29.4% | 9.4% | 0.1 / 0.0; 1.2 / 10.2; 0.1 / 15.9% | 99.3 / 99.2; 84.0 / 64.4; 81.3 / 81.5% | 0/4 (v3: 3) |

EVIDENCE: SR/finder/out/<repo>.summary.json (keys `classes_qualifying`, `functions_qualifying_t2_pct`, `function_body_bytes_qualifying_pct`, `member_access_qualifying_pct_of_dynamic`, `member_access_sound_in_region_pct_of_dynamic`, `modules_ge50pct_v0`); the in-region key counts accesses whose receiver is a Q class and that sit inside a Q function (SR/finder/analyze.py:800-801). SR/hot/res/table.md; SR/hot/res/<workload>[-noinl].json. Ladder: v1 = `!` becomes a null check and class casts become brand checks; v2 = + lenient constructor rule; v3 = + generic T allowed. v1-v3 regions keep these checks inside the region. The AWFY port was written for ep9 [R13] (13-tsrs-type-oracle.md:9; its DeltaBlue sort differs, :113), and docs/plan/05-aot.md:178 says typed AWFY "does not count" for the real-app off-ramp. It is the best case by construction, not evidence that regions are found in user code.

**Approximations (direction).**

| Approximation | Direction |
|---|---|
| The finder applies TS-type rules (the A5 subset). The real verifier proves facts from JS semantics plus B and F. A typed param still needs an entry check; an interface param with few runtime shapes could pass by B | Both |
| The finder checks only params, locals, `this`, casts, any-flows, r2 and r7 (SR/finder/analyze.py:579-630). It does not check callees, globals, or member chains on unbound expressions. A missing type counts as strict (`tier_of(None)` returns 0, analyze.py:582-583; how often: not measured) | Over-count |
| Q functions with 0 params and 0 locals pass with nothing checked: tsc 723/1619 (45%), hono 107/260, vscode 657/1709, zod 66/159 (summary.json: `functions_qualifying_t2` minus `functions_qualifying_nontrivial`) | Over-count (time share not measured) |
| Closed world: walked files only. In open world, qualifying classes drop to 0-62 per repo (hono 0, date-fns 0, vscode 62) | Over-count |
| Captured outer variables are not checked. Untyped mutable captures: tsc 2.3 of 6.7 and 2.6 of 8.4 points. Untyped const captures (binding fixed, value shape unknown): tsc 1.1-1.3 points; date-fns 4.0 of 5.7 and 5.8 of 10.9; vscode-diff 8.5 of 23.9 and 4.5 of 39.1 (SR/hot/res/capweight.json; its Q totals differ from table.md by up to 0.2 points) | Over-count |
| `r7` matches eval/Function by syntax. zod's `const F = Function; new F(...)` (zod core/doc.ts:44-46) escaped it | Over-count (zod is 0% anyway) |
| Element-shape facts (`arr[i]` is a class instance) are not modeled. In DeltaBlue, `class Variable[]` is the first blocking type for 27.3% of time and `class AbstractConstraint` for 21.3% (SR/hot/res/awfy-deltablue.json, `bad_type_time_pct`). Per-load checks in kernel regions are not counted | Over-count of guard-free code |
| Hot shares come from V8 profiles, not ep9. With V8 inlining on, Q leaf functions merge into non-Q callers | Unknown in ep9 |
| Array OOB, bivariance, covariant arrays, int vs double, stale narrowing ([R13] H2/H5/H6/H8/H13) are not rules. So bounds, hole and int32 checks may remain inside regions | Over-count of guard-free code |
| All coverage comes from TS-type rules on TS sources. With D18 = c (no ep9-typecheck, docs/plan/11-decisions.md:114) or on bundled input ("Bundled input gets no hints", RK39, docs/plan/10-quality.md:231), the verifier works alone; bundler-lowered class fields may use defineProperty, a breaker | Unknown; not measured |
| Sampling error: about ±0.7 points at a 10% share with 4.8k samples; tsc per-run spread 6.4-6.9% (default) | Noise |

## 3. Precedents

| System | What it did | World | Result | Fate |
|---|---|---|---|---|
| SpiderMonkey TI (shipped Firefox 9-82) | Lazy run-time inference ("unsound static inference ... with targeted dynamic type updates") + type barriers (guards) + invalidation | Open | vs JaegerMonkey without TI: V8bench 1.50x, Kraken 2.69x, SunSpider 1.06-1.27x. Barriers on 41.4% of property/element accesses on average: websites 35-74%, v8bench 26%, Kraken 3% | Disabled in Firefox 83 (Warp), code removed from Firefox 85 (about 50,000 lines). Warp: Speedometer +10-12%, Docs load +20%, memory -8%; but slower than Ion+TI on Octane and Kraken |
| JSC InferredType (2015-2019) | Automatic type per field + watchpoints | Open | Removal: Speedometer2 +1%, JetStream1 -1% | Removed |
| V8 / JSC / SM dependencies today | Narrow, on-demand facts, each with invalidation | Open | Not isolated. E3: `--no-track-field-types` (heap-object field class only) geomean 1.00x | Alive. ep9 H31 copies this model |
| Hopc (DLS'18, ICFP'21) | Automatic AOT: generic + specialized copy per function | Open; separate compilation | 0.59-7.31x of V8 time (32-bit build): base64 0.59x, richards 3.34x (0 typed calls of 902k), deltablue 6.81x (80k typed vs 82M untyped calls; also GC-bound) | Research. Automatic per-function specialization covered few calls on the OO kernels that A5v counts on |
| Hopc sealed (ECOOP'22) | Opt-in `// @sealed` classes; rules C2/C3 (not extensible, immutable prototypes) enforced, violations raise errors | Module (separate compilation; class hierarchy must be known, C1) | 0.44-0.98 of IC-based Hopc time (geomean about 1.27x) | Research |
| Static Hermes | Untyped: local inference only. Typed: opt-in Flow subset, THROW | Module | E1: typed 3.3-5.4x faster than the same source untyped; typed still 1.5-3.2x slower than V8 [R15] | Typed mode "unstable" |
| asm.js | Opt-in subset; link check; on failure the same code runs as plain JS | Module | n/a | Superseded by Wasm |
| HHVM RepoAuthoritative + HHBBC | Whole-program inference, valid because "no more code can ever be brought into existence" | Closed; `eval()` and `create_function()` not supported | n/a | Ships; opt-in deploy mode |
| Closure Compiler type-based optimizations | Opt-in `--use_types_for_optimization`; relies on unchecked assumptions (for example, property reads have no side effects; builtins not redefined) | Closed per compile | n/a | Ships; unsound by design |
| Dart TFA, GraalVM native-image | Whole-program sound inference | Closed; entry points must be listed | Dart polymorphic code 1.94x JIT time | Ship; not possible for JS in test262 mode |
| TAJS, WALA, SAFE | Sound whole-program JS analysis | Closed | TAJS: 86 of 154 jQuery programs in 1 min; WALA: no jQuery >= 1.3 | Research |
| V8 strong mode | Opt-in sound mode | Script | "did not materialise" | Removed 2016 |
| Numba nopython | Automatic, silent fallback to object mode | Function | Performance cliffs | Fallback removed in 0.59; failure to compile in nopython mode is now an error. The opposite of a silent automatic model |
| Higgs BBV | Runtime block versioning, no static analysis | Open | Removes 94.3% of type-tag tests | Research |

**The SpiderMonkey TI lesson, plainly.** TI was not guard-free and not static: it inferred types lazily at run time and needed barriers on 41.4% of accesses on average. It shipped from Firefox 9 (2011) to Firefox 82; Firefox 83 replaced it with Warp. TI won most on benchmark kernels (Kraken 2.69x, with barriers on 3% of accesses). Websites needed barriers on 35-74% of accesses. Its costs fell on all code: type tracking in every function, memory, GC time, over-specialization ("many bailouts") and type-confusion bugs (CVE-2019-9791). Warp gave up some kernel wins (slower on Octane and Kraken) to remove those costs. A5v targets the same kernel niche. For ep9 this means: prove offline, keep facts narrow and per region, add no cost to code outside regions, and never build a heap-wide type-set system.

EVIDENCE: SR/precedents/dl/ti-pldi12.txt:26 (Firefox 9), 102-104 (hybrid inference), 271 (lazy analysis), 460-472 (speedups), 548-575 (barriers, Figure 9) (https://rfrn.org/~shu/papers/pldi12.pdf); SR/precedents/dl/warp.txt:16, 106-116 (bailouts at :110) (https://hacks.mozilla.org/2020/11/warp-improved-js-performance-in-firefox-83/; Octane/Kraken at :112); SR/precedents/dl/2020_12_18_newsletter-8.txt:45; SR/precedents/dl/wk190906.xml:410-413 (https://bugs.webkit.org/show_bug.cgi?id=190906); SR/precedents/dl/dls18.txt:645-661 (Figure 7), 669-673 (deltablue GC), 705, 711 (Figure 8); SR/gain/dl/sealed.txt:525-560 (C1-C3), 1162-1176 (Figure 10, baseline is unmodified IC-based hopc) and sealed.pdf p.18; SR/refute/dl/repo-auth.html (https://docs.hhvm.com/hhvm/advanced-usage/repo-authoritative); SR/refute/dl/hhbbc.h:92; SR/precedents/dl/cc-wiki/Compiler-Assumptions.md:17-26 and Type-Based-Property-Renaming.md:9; SR/precedents/dl/tajs-jquery.txt:760-807; SR/precedents/dl/numba-dep.rst:155-195; https://groups.google.com/g/strengthen-js/c/ojj3TDxbHpQ/m/T6bTRDlnEwAJ; https://arxiv.org/abs/1511.02956.

## 4. Expected gain per mode

| Mode and comparator | Inside a region | End to end (Amdahl, hot share from §2) | Conf. | Basis |
|---|---|---|---|---|
| JIT, vs T2 with feedback | +2-8% (guards only); up to +20% on FP/class kernels | tsc +0.3-0.4%, vscode-diff +1-2%, kernels +3-5% (est., at 1.05x) | medium | IISWC'16 2.2-4.6%, StrongScript -0.9% to +22% [R12]; TI vs Warp |
| Hybrid, peak | same as JIT | same as JIT | medium | the JIT re-tiers hot code (H33) |
| Hybrid, first iteration | maybe better (no early guard failures) | not measured | low | - |
| AOT-jitless, vs A4 optimized-AOT (guarded, PROFILED + HINTED) | Class-heavy code: up to about 1.27x. This is an upper bound: Hopc sealed enforces its rules with errors and is measured against IC-only Hopc. Code where almost no member access is in a region on a Q receiver (tsc, hono, date-fns: 0.1-0.2%): guard-only band +2-8% | tsc +0.1-0.7%, hono +0-0.5%, date-fns +0.1-0.8%, vscode-diff +0.5-10.2% (top end at 1.27x), kernels (v3, with inner guards) up to +16-27% (est.) | low | Hopc sealed; IISWC'16 band |
| AOT-jitless, vs baseline-AOT | 3-5x upper bound; ep9 baseline-AOT has data ICs, so less | tsc +5-6%, vscode-diff +19-35% (est., at 3x) | medium | E1: Static Hermes typed vs same source untyped |
| vs V8 full at peak (FV3) | nbody-type code 1.1-1.2x faster; OO code 0.6-1.0x | kernels only | medium / low | scriptc nbody 1.15x, C 1.22x [R15][R17]; no OO win anywhere |

EVIDENCE: E1 = SR/gain/sh-typed-vs-stripped.log (nbody 940 vs 195 ms, richards 930 vs 284, raytracer 802 vs 148; checksums match). E2 = SR/gain/emit-counts.txt (untyped C: 0 direct slot loads). E3 = SR/gain/v8flags.log (best of 3; noisy). Amdahl: speedup = 1 / ((1 - s) + s / g), with s from SR/hot/res/table.md; all values in SR/revise/amdahl.log (est.).

## 5. Findings

1. **[medium] The idea is sound in full ECMAScript if the fuse and effect-point lists are complete. Three rules: every fact has S, B or F; failure runs generic code; types only propose.** Each of the 30 breaker probes (B01-B28c) is covered by a rule (R1-R8), S, B or F in the §1 tables. The 24 [R12] hole classes are cases where TS types lie; R2 never trusts types, so they do not test whether S/B/F is complete. Similar systems had misses: TI (CVE-2019-9791) and a V8 map-deprecation dependency (CVE-2020-16009). Known gaps found in this round: interrupt polls (R5), resizable-buffer shrink (F6), element shapes (§1 fact table) and PROVEN locals under a debugger (B28c). Out of reach: SharedArrayBuffer values, raw native memory and "never throws" facts. The per-class S/B/F split of the semantics study was not saved and is not re-checked here.
   EVIDENCE: §1 tables; SR/semantics/probes.log:1-30 (node 24.22); SR/refute/probes.log:3; Finding 12 links.
2. **[high] Class-level facts need fuses, not static proofs.** 55.6-100% of classes are exported. Any escaped instance exposes its class through `.constructor`. In open world, qualifying classes drop to 0 (hono, date-fns) or 62 of 761 (vscode).
   EVIDENCE: SR/semantics/scan.log:11-17; SR/finder/out/*.summary.json key `classes_qualifying_open_world`.
3. **[high] Static coverage is small.** Without the AWFY port: 9-23% of functions and 1.7-14.7% of function-body bytes qualify. 0.4-16% of dynamic member accesses use a qualifying receiver, and only 0-6.2% are also inside a qualifying function. (ep9's AWFY port: 18.6%, 15.2%, 29.4% and 9.4%.)
   EVIDENCE: §2 table; SR/finder/out/*.summary.json.
4. **[high] Hot time in qualifying functions is small in real library and compiler code.** tsc 6.7-8.4%, hono 0.2-6.2%, zod 0%, date-fns 5.9-11.0%. Only the vscode diff engine (23.9-39.1%) beats its static function share (23.0%); tsc (14.6%) and date-fns (13.7%) stay below theirs. tsc's profile is flat: 135-151 functions make 50% of analyzed time.
   EVIDENCE: SR/hot/res/table.md; SR/hot/res/tsc.json (`functions_for_50pct_of_analyzed_time`).
5. **[high] The hot blockers are structural.** 41-48% of tsc time has `Type`, `Node` or `Symbol` (or arrays and nullable forms of them) as the first blocking type. These are open interfaces for objects made by plain function constructors in `objectAllocator` (TypeScript src/compiler/utilities.ts:8471-8552), not ES classes. Function-typed values are the first blocking type in 0-19.1% of time (zod 6.7%, DeltaBlue with inlining off 19.1%, others <= 1.8%); a full callback what-if was not run (§7). Non-qualifying `this` classes (constructor order r4 and other class rules) and casts block much of the rest: a non-qualifying `this` class is the first blocker for 50-51% of hono-routers time, and casts appear in 43% of tsc time.
   EVIDENCE: SR/hot/res/*.json (`bad_type_time_pct`, `blocker_time_pct_first`, `blocker_time_pct_any`); SR/hot/allfuncs/typescript.ifaces.json.
6. **[high] Relaxations help kernels, not apps.** Guarded `!`/casts, a lenient constructor rule and generic T take ep9's AWFY port from 0-16% to 64-99% of time. The same steps move tsc only from 8.4% to 8.8%. One constructor case (`NBodySystem.bodies` set after `this.createBodies()`) holds 63.5% of NBody time.
   EVIDENCE: SR/hot/res/table.md; SR/hot/analyze_all.py:580-650 (what-if).
7. **[high for shares] Region code must stay inlinable.** With V8 inlining on, the Q-v0 share drops: vscode 39 to 24%, DeltaBlue 10 to 1.2%, NBody 16 to 0.1%. A region compiled as a separate function keeps a call cost unless non-region callers can inline it.
   EVIDENCE: SR/hot/res/*-noinl.json vs *.json.
8. **[medium] JIT mode gains little.** V8 at peak without heap-object field class tracking (`--no-track-field-types`) ran at geomean 1.00x (noisy). The flag only gates `FieldType::Class(map)` for heap-object fields; Smi and Double representation tracking stays on, so E3 says nothing about unboxed or int32 facts. Without inlining V8 ran 1.10-3.30x slower, and without load elimination up to 1.59x slower. So in V8, inlining and load elimination matter more than field class facts.
   EVIDENCE: SR/gain/v8flags.log; SR/refute/dl/objects.cc:224-238 (V8 13.6 `Object::OptimalType`); TI vs Warp (§3).
9. **[medium] AOT-jitless mode gains more.** Static Hermes typed native code is 3.3-5.4x faster than the same source untyped. Its untyped C has 0 direct slot loads. Hopc sealed classes (opt-in, rules enforced by errors) run in 0.44-0.98 of IC-based Hopc time.
   EVIDENCE: SR/gain/sh-typed-vs-stripped.log; SR/gain/emit-counts.txt; SR/gain/dl/sealed.txt:1162-1176 and sealed.pdf p.18.
10. **[medium] Whole-bundle sound analysis is a bet. Function and module scope are feasible.** TAJS finished 86 of 154 small jQuery programs in 1 minute. Hopc's full compile of moment.js took about 5 minutes; that is backend time (the JS -> Scheme -> C chain grows the code 50-100x), not analysis time. The finder's fact export (tsrs check plus export, not a verifier) took 17.6-1180.8 ms per repo.
    EVIDENCE: SR/precedents/dl/tajs-jquery.txt:760-807; aot-precedents/docs/icfp21.txt:392-400 [R10]; SR/finder/out/*.log (`ms_total`: AWFY 17.6 ms, tsc 1180.8 ms; computed at SR/finder/facts/src/main.rs:1574-1663).
11. **[high for V8 behavior] A debugger breaks facts that no static rule protects.** The in-process `node:inspector` retyped a field of a never-escaping object (B25). A worker of the same program rewrote a strict `let` and a `const` through `Debugger.setVariableValue` (B28, B28c). A worker session's `Runtime.evaluate`, with no domain enabled and no `debugger` statement, ran inside a call-free loop and retyped a field (new probe). So F7 must pop when a session connects, and every interrupt poll must be a lazy-deopt point. The PROVEN-locals part is a defect for all tiers, not only for A5v (§6.0).
    EVIDENCE: SR/semantics/probes.log:26,29,30; SR/refute/interrupt-probe2.mjs; SR/refute/probes.log:3; SR/revise/probe2-rerun.log; docs/plan/02-architecture.md:180-181 (H23, H24); docs/plan/04-execution.md:194-198.
12. **[high] Guard-free code turns analysis bugs into type confusion.** TI had CVE-2019-9791. V8 missed a map-deprecation dependency in CVE-2020-16009 (exploited). Since 2020, V8 keeps bounds checks as aborting checks. ep9 should do the same, which lowers the "no guards" gain. With aborting checks, a missed fuse changes output: a missed resizable-buffer shrink (B15) aborts the process where JS returns `undefined`.
    EVIDENCE: https://nvd.nist.gov/vuln/detail/CVE-2019-9791; https://googleprojectzero.github.io/0days-in-the-wild/0day-RCAs/2020/CVE-2020-16009.html; https://projectzero.google/2021/01/in-wild-series-chrome-infinity-bug.html; SR/semantics/probes.log:16.
13. **[high for the precedent] A silent fallback causes performance cliffs.** Numba removed its automatic fallback for this reason. ep9 needs a region report ("qualified", or "not qualified, because ..."). Its form is a product call.
    EVIDENCE: SR/precedents/dl/numba-dep.rst:155-195.
14. **[high] Code generated at run time is outside any static finder.** 12.5-13.3% of zod's time runs in code that zod builds with `new F(...)`. Only JIT mode can optimize it.
    EVIDENCE: SR/hot/res/zod-parse.json; zod packages/zod/src/v4/core/doc.ts:44-46.

## 6. Plan delta proposal

Status 2026-10-20: decided ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)). D17 gains option (d) under a rule; SP-RegionCoverage runs in A1h; K-A5v-cov stays at 15%; K-A5v stops only if both parts fail; a kernels-only A5v needs ≥ 1.3x A4 code; the H24 debugger rule is in 02-architecture.

**6.0 Changes now (before any A5v data)**

| Item | Change |
|---|---|
| New pre-check SP-RegionCoverage | <= 1 PM (est.), any time before M7. Reuse the finder and hot mapper of this round. Run on the 3 real-app HINT-SET groups, TYPED-KERNELS and one vscode-diff-style workload. Report the hot share in Q functions, the in-region member-access share, a types-off variant (D18 = c) and a shipped-bundle variant (RK39). Kill rule K-A5v-cov (§6.4) |
| D17 (docs/plan/11-decisions.md:113) | Stays "Defer to M7 exit". Add option (d): "automatic VERIFIED regions: no opt-in, no edits, no THROW, test262 mode". Today's data limits (d) to kernels: K-A5v-cov trips on tsc and hono |
| Separate item, not A5v: PROVEN locals vs debugger | B28 and B28c show that a session rewrites strict `let` and `const` locals. H24 needs a rule for PROVEN facts on locals while a session is connected. File it as its own plan item for the H24 owners |

Everything below applies only if the D17 rule picks (d) at M7 exit: K-A5v-cov passed, or region code ≥ 1.3x A4 code on ≥ 2 of 4 TYPED-KERNELS (kernels only).

**6.1 Trust levels (docs/plan/05-aot.md §4, 04-execution.md §6-§7)**

| Level | What goes in | Guard | On failure | Where |
|---|---|---|---|---|
| PROVEN | unchanged (see the §6.0 item) | none | - | every mode |
| VERIFIED (new) | Facts that the ep9c verifier proves per region from bytecode, module graph and the builtin effect model, under checks and fuses. Types only choose candidates | Entry checks; hoisted interior checks; aborting checks; F dependencies | Entry fail: generic sibling (SLOWPATH), jitless budget then PIN. Interior fail: DEOPT with frame state. Fuse pop: lazy deopt + slot swap. Never THROW | AOT units (jitless; hybrid until re-tier); test262 mode included |
| HINT | unchanged | unchanged | unchanged | unchanged |
| SOUND (opt-in, THROW) | frozen; built only if the D17 rule picks (c) at M7 exit (decided 2026-10-20) | - | - | - |

H30 merge rule: key VERIFIED facts by region version, not by bytecode site. Inside a region version, VERIFIED ranks above RUNTIME while its fuses hold; a conflict there means a missing fuse pop, so it is an engine bug (debug assert). At the same bytecode outside the region, RUNTIME feedback may legally differ: the generic sibling runs inputs that failed the entry checks, and baseline-AOT ICs write the shared feedback (H33, docs/plan/02-architecture.md:190).

**6.2 Stages and timing**

| Item | Today | Proposed |
|---|---|---|
| A5 = M10 | Opt-in SOUND modules, THROW, outside test262, 12-24 PM, after M7 (docs/plan/05-aot.md:357) | A5v: automatic VERIFIED regions in ep9c AOT units. After M7 (needs T2 passes) and on the A4 T2 AOT emit path (depends on K-A4J). FV3 part needs M9. No JIT-mode region table (JIT gain est. +0-2%) |
| SP-TypedBound (M7) | T2 with guards removed on TYPED-KERNELS | + SP-RegionBound: remove guards only inside finder regions (SR/finder/regions.json), in JIT mode and in jitless mode (vs A4 code). Count entry, hoisted interior and aborting checks per region. Add a SIZE-REPORT line: unit text with the second code copy vs the A3 gate (unit text <= 8x bytecode, docs/plan/05-aot.md:355). +0.25 PM (est.) |
| Early contract items | - | None. H17 stays as it is before M1a unless Spike C or SP-Facts measures a gain. Decide the field-type lattice after SP-RegionCoverage passes |

**6.3 D15 and D17 wording (docs/plan/11-decisions.md:111, :113).** D17: see §6.0. If K-A5v data later passes, D17 becomes "is A5v worth 16-33 PM?" instead of "may ep9 change JS semantics?". D15 (b) wording ("found automatically" instead of "opt-in") is a product claim near N10; decided 2026-10-20: mechanism-neutral wording (11-decisions D15).

**6.4 Kill rules (replace K-A5, docs/plan/05-aot.md:371)**

| ID | Checked | Rule (est.; thresholds decided 2026-10-20) | Today's data |
|---|---|---|---|
| K-A5v-cov | SP-RegionCoverage | hot-time share in finder-v1 regions < 15% on >= 2 of the 3 real-app HINT-SET groups (TS compiler, vscode, hono; docs/plan/05-aot.md:177) -> limit A5v to kernels or stop | tsc 7-9%, hono 0.2-6%: trips; vscode-diff 24-39%: passes |
| K-A5pre | M7 | unchanged (< 1.15x -> stop), but it gates only the FV3/JIT part. It measures JIT mode; A5v's claim is in jitless mode | not measured |
| K-A5v-bound (new) | SP-RegionBound, jitless | region code < 1.1x A4 code inside regions on TYPED-KERNELS -> stop | not measured |
| K-A5v | after the prototype | Real-app part: < 3% geomean end-to-end gain on the 3 real-app HINT-SET groups (typed AWFY excluded, as in the K-A4H off-ramp), AOT-jitless vs A4 optimized-AOT -> no real-app claim; A5v is kernels only. Kernel part: FV3 not met -> no D15 (b) claim. Both fail -> stop | Real apps +0.2-3.7% (est., §4 bands). At 1.27x for all groups: real apps +2.3-4.4%; with typed AWFY included +5.5-9.6% (SR/revise/amdahl.log) |
| Dependency | K-A4J (M8 exit) | If K-A4J trips, A5v loses its codegen path (T2 AOT emit) and its comparator -> stop A5v (D17 rule: a K-A4J trip stops (d), decided 2026-10-20) | - |

**6.5 FV3.** Keep the gate (>= 1.1x V8 full on >= 2 of 4 TYPED-KERNELS). Note: TYPED-KERNELS are typed ports (docs/plan/10-quality.md:95); the AWFY Richards, DeltaBlue and NBody ports were written for ep9 [R13]. AWFY reaches high coverage only with v1-v3 relaxations, so guards remain inside regions. Only nbody has data for a win [R15][R17].

**6.6 H rows touched**

| Row | Change | PM (est.) |
|---|---|---|
| H30 | VERIFIED level, region id and fuse list per fact, facts keyed by region version, new merge rule | 0.5-1 |
| H31 | New fuses F1-F8 (§1): per-key prototype conditions (incl. AbsenceOfSetEffect), HasPrototype, shape stability, field type, array protectors, buffer detach and shrink, inspector session (F7), closed world (F8, optional); arm-time validation; realm-relative fuse refs | 4-8 (shared with T2) |
| H32 | Rows: VERIFIED entry fail (SLOWPATH), interior check fail (DEOPT with frame state), aborting check, fuse pop. The SOUND THROW row becomes unused | 0.25 |
| H17 | Only if a gain is measured (§6.2): shape-stability bit, field-type lattice per descriptor, per-key prototype conditions | 2-4 (shared with T2) |
| H23 / H24 | F7 pops when an in-process or worker session connects. Every interrupt poll (loop back-edge, function entry) is a lazy-deopt point that re-reads F7. No VERIFIED facts while a session is connected. (PROVEN locals: §6.0 item) | in H31 |
| H16 / H33 / H35 | FactSnapshot VERIFIED entries; region table, link cells and arm-time records in the unit | 1.5-2.5 |
| H37 | Firewall: ep9_hints feeds the finder only | 0.25 |
| H38 (new) | Builtin effect model (may call user code / may mutate / may throw), generated from the spec layer, differential-tested | 1-3 |
| ep9c pass | New offline analysis pass: finder + verifier, bytecode SSA, range analysis, construction model, module escape analysis, region versioning, region report | 6-12 |
| Tests | Fuse chaos, inspector chaos (incl. interrupt-poll evaluate), entry-check chaos; hint chaos must not change the VERIFIED set | 1-2 |
| **Total** | vs A5 prototype 12-24 PM | **about 16-33** |

The total includes H31 (4-8) and H17 (2-4), both marked "shared with T2". If T2 already budgets them, A5v's own cost is about 10-21 PM (est.).

**6.7 New risks (docs/plan/10-quality.md, after RK44)**

| ID | Risk | Mitigation | Trigger |
|---|---|---|---|
| RK45 | A verifier or effect-model bug, or a missing fuse, becomes type confusion in guard-free code, or an abort at an aborting check (B15) | H38 generated from the spec layer; aborting bounds checks kept; chaos fuzzing; mode x tier differential | any chaos diff |
| RK46 | Fuse storms: polyfills, mixins and APM tools pop fuses, so regions never run | per-key conditions; arm after module-graph init; F7 pops on session connect (any domain can run code); pop counters in reports | arm failures or pops on HINT-SET |
| RK47 | Silent fallback gives performance cliffs | region report in ep9c (form: product call) | user reports |
| RK48 | Regions miss the hot code | SP-RegionCoverage; K-A5v-cov | coverage under threshold |
| RK49 | Field-type tracking costs store paths and memory in all code (TI's cost) | F4 optional; measure in Spike C / SP-Facts before M1a | T0/T1 regression > 2% (est.) |
| RK50 | Build-time analysis is slow on large bundles | function + module scope; bundle scope (F8) optional and time-boxed | analysis time over budget on HINT-SET |
| RK51 | Coverage depends on TS types and unbundled sources | SP-RegionCoverage types-off and shipped-bundle variants; A5v needs D18 != c unless the types-off variant passes | types-off or bundle coverage under threshold |

## 7. Gaps and the cheapest experiment for each

| Gap | Not measured because | Cheapest experiment |
|---|---|---|
| Verifier precision vs the TS-type proxy | no verifier exists | Module-scope escape + range analysis on oxc semantic for the top 50 hot functions per workload; compare with the finder v0 set (2-4 weeks, est.) |
| Coverage with types off and on shipped bundles | the finder uses TS types on TS sources | SP-RegionCoverage variants: JS-only facts on each app's shipped bundle (days) |
| A typed kernel not written for ep9 | the typed AWFY port is ep9's own [R13] | Rerun the finder and hot mapper on a third-party typed TS kernel set (days) |
| Element shapes in kernel loops (nbody bodies, DeltaBlue variables) | not modeled | Count class-typed element loads in hot kernel loops; try module-closed points-to on nbody and DeltaBlue (days) |
| Gain inside regions on ep9 | no T2 yet | SP-RegionBound at M7 (+0.25 PM), with interior-check counts and SIZE-REPORT. Before that: nothing cheap and faithful |
| Hot shares under a jitless engine | profiles came from V8 with JIT | Re-run SR/hot/runall.sh with `node --jitless` profiles (hours) |
| Fuse pop rate on real apps | no ep9 runtime | `node --trace-deopt` on HINT-SET; count dependency deopts (field type, prototype chain, stable map) inside functions in SR/finder/regions.json (days) |
| Allocation-site layouts for tsc's `Type`/`Node`/`Symbol` (41-48% of tsc time) | finder proves by ES class only | Wrap the `objectAllocator` constructors in a node script; record own keys at allocation and at exit per constructor (days) |
| Callback parameters as opaque call-outs (saved data: function types are the first blocker in 0-19.1% of time) | rule not built | What-if run of SR/hot/analyze_all.py with fn-typed params treated as effect points (hours) |
| Region code inlined into non-region callers | design question | Decide in the ep9_opt ADR: inlined region code keeps guards (facts degrade to PREDICTED) |
| Bundle-scope analysis time | no points-to prototype | Run the finder over all of vscode src and time it; then a module-scope Andersen prototype (1-2 weeks, est.) |
| Store-path cost of F3/F4 in a NaN-boxed engine | no ep9 object model | Add it to Spike C / SP-Facts (M0-M1) |
| pixijs and preact hot time | no driver | One render-loop driver for pixijs (days) |
| Hopc sealed per-test values | read from Figure 10 bars | Rerun Hopc, or ask the authors for the data |
