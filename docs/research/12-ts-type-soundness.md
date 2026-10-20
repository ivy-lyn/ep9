# ts-type-soundness

## Summary
You can't trust plain TypeScript types for codegen without a guard. TS is unsound and erased by design. I wrote 24 small strict-mode programs. tsc 6.0.3 --strict, tsc 7.0.2 and tsrs 0.18 (TS 7.1-dev port) all accept every one with 0 errors, yet at runtime they produce 47 values that contradict their static types: any, as-casts, `!`, method-parameter bivariance, array and index-signature covariance, out-of-bounds/Record reads, tuple push, optional-property re-add, lying .d.ts, holes, readonly getters, defineProperty, setPrototypeOf, prototype patching, Proxy, direct eval, field read before super() returns, numeric enums accepting any number, generic casts, unbound this, overload implementations, narrowing kept across calls, lying type guards and `asserts`. The strictest flags (noUncheckedIndexedAccess, exactOptionalPropertyTypes) close only 3 of the 24.

I measured 6 real codebases (TS compiler, zod, date-fns, hono, pixijs, vscode base+editor; about 344k code lines) with the TS 6.0.3 checker API:
- **Ints:** types prove essentially no int32s. Only 1-9% of non-constant numeric expressions have an int-literal or enum type. The exception is the TS compiler (37%), whose enum types are unsound anyway.
- **Layout:** the TS compiler and zod access properties almost entirely through interfaces or structural types (only 0.4% / 6.4% of dynamic a.b go through class receivers). pixijs and vscode use classes for about 60%. So types give no layout for most code.
- **Escape hatches:** 3-42 casts per KLOC plus 0.1-7.6 `!` per KLOC.
- **Settings:** pixijs runs with strictNullChecks OFF. 4 of 6 codebases compile class fields with [[Set]] semantics, while node's type stripping uses [[Define]]. I showed this gives different object shapes for the same source.

The literature agrees with this:
- Making TS types sound costs 6.5-15% (Safe TypeScript) or needs a restricted dialect: Static Hermes (exact objects, nominal classes, no holes, bounds checks, final accessors), Static TypeScript, StrongScript.
- The speedup over a speculative JIT at peak is small: StrongScript in V8 was -0.9% to +22% on 5 class-heavy Octane benchmarks. Removing all of V8's deopt checks gains only 2.2-4.6% (IISWC'16); an IISWC'21 abstract puts checks at about 8% on JetStream2.
- V8 dropped strong mode / SoundScript: 'our hope for being able to utilise strong mode for improving performance did not materialise'. SpiderMonkey removed its global Type Inference and got faster on real workloads with feedback-only Warp: +10-12% Speedometer, +20% Google Docs load, -8% memory.
- The big wins come where there is no JIT or no warm-up: Dart 2 AOT, Static Hermes typed native at 9.8x its interpreter on nbody, mypyc 1.5-5x over CPython, Luau's checked annotations.

In my tsc self-check run on V8 there were 1,693 deopts:
- 49% were 'wrong map' (shape failures that TS interface types can't prevent);
- 33% were 'insufficient feedback' (the only gap that ahead-of-time types could fill);
- 0.4% were 'not a Smi' (int32 speculation almost never fails, so TS's lack of int info doesn't matter).

## Recommendation
Treat TS types as free ahead-of-time feedback, never as facts. Three tiers of use:

```
                       what TS types give           runtime check needed               gain vs good spec. JIT
SOUND (no check)       nothing beyond what ep9's     -                                  0
                       own JS flow/range analysis
                       proves anyway
GUARDED SPECULATION    param/return/field repr      entry guard per value (tag test),  peak: ~0-5% (guard-removal
(AOT, deopt to         (f64/bool/string/obj);       shape check on class receivers,    ceiling 2-8%);
interp/baseline)       class receiver -> shape      elements-kind check hoisted at     warm-up / JIT-less:
                       guard + slot offset;         loop entry, null check for `!`;    large (Static Hermes
                       non-null; number[] ->        on failure deopt, not throw (JS    9.8x vs interp; Dart AOT)
                       packed double elements;      semantics must win)
                       cold-path feedback seeding
CLOSED WORLD /         devirtualization (CHA +      none at use sites, but needs a     +6..22% on class/float
OPT-IN SOUND DIALECT   final), fixed field layout,  restricted dialect + checks at     kernels (StrongScript);
                       unboxed fields, monomorph.   every any/JS boundary (Safe TS     Safe TS cost 6.5-15%
                       generics, cross-module       6.5-15%), watchpoints on proto/
                       inlining, no-alias readonly  defineProperty/eval
```

Concrete plan for ep9:
1. Run tsrs as the oracle, out of process or as a build step. It is fast enough: 0.11-0.31 s for 264k lines. Map tsrs positions (u32 byte offsets that include trivia; skip_trivia to the token start) to oxc spans. For each site, emit hints: expected representation, expected class (a nominal hint only when the class has #private or private members or is final-like), non-null, and array element type.
2. Use the hints in two ways. (a) Pre-fill the feedback vector, so cold paths stop deopting for 'insufficient feedback'; that was 33% of V8 deopts on tsc. (b) Seed an AOT/baseline tier with guarded specializations that deopt into the interpreter on failure, the way Luau does. Never throw on a mismatch, because types are erased in JS.
3. Keep runtime feedback authoritative for int32 vs f64, shapes/maps, call targets, holey vs packed, and constant fields. TS cannot express these: 'wrong map' was 49% of deopts, while 'not a Smi' was 0.4%.
4. If the user wants the 'deep optimizer' AOT, ship it as an opt-in sound subset modelled on Static Hermes/STS: exact objects, nominal classes, final methods/accessors, no holes, bounds-checked arrays, checked casts at any/JS boundaries, no eval/with/Proxy on typed objects. Gate it per module, and honor the tsconfig flags useDefineForClassFields and experimentalDecorators, because they change runtime shapes.
5. Expect at most about 0-5% at peak over a Maglev/DFG-class JIT on ordinary TS. The real wins are startup/warm-up, JIT-less targets (iOS) and memory (fewer feedback vectors and recompiles), plus about 10-20% on class- or float-heavy kernels in sound mode. Don't let 'TS types make AOT faster than V8' drive the architecture.

## Findings
- [high] No TS type is a sound basis for unchecked codegen: 24/24 strict-mode programs type-check with 0 errors in tsc 6.0.3 --strict, tsc 7.0.2 and tsrs 0.18, and at runtime produce 47 values that contradict their declared types.
  EVIDENCE: Suite: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/unsound/src/c01..c24*.ts with tsconfig strict, target es2022.
Commands: `node node_modules/typescript/bin/tsc -p src` (exit 0), `node node_modules/ts7/bin/tsc -p src --noEmit` (exit 0), `target-tsrs/release/tsrs -p src --noEmit` (exit 0; 'Version 7.1.0-dev (tsrs 0.18.0-ts7.1.0-dev.20260929)'). Running the output prints 47 'LIE' lines (unsound/run.log).
Examples:
- JSON.parse any->number: runtime string
- `{} as Point`: p.x undefined
- `m.get('missing')!`: undefined
- method-syntax and class-override parameter bivariance: TypeError d.bark
- `Dog[]` aliased as `Animal[]` + push(cat): crash
- arr[10]: undefined; tuple [number,number] after push has length 3
- `{a:number,b:string}` -> `{a:number}` -> `{a:number;b?:number}`: b is a string (still accepted with exactOptionalPropertyTypes)
- lying legacy.d.ts
- new Array<number>(3) holes
- readonly getter returns a different value on each read; readonly alias gets mutated
- Object.defineProperty / setPrototypeOf / Object.assign(P.prototype) retype fields and methods
- Proxy get trap
- direct eval writes a local `x:number` = 'str'
- derived field read during super(): undefined
- `const c: Color = 42`
- `cast<T>`
- extracted method has undefined this
- overload implementation returns a string where the signature says number
- narrowing survives mutate(): box.v is 42 where the type is string
- lying `x is number` and `asserts x is string`
- [high] The strictest optional flags close only 3 of the 24 holes, and do it by making types less informative: with noUncheckedIndexedAccess, every arr[i] becomes T|undefined.
  EVIDENCE: `tsc -p src --noEmit --noUncheckedIndexedAccess --exactOptionalPropertyTypes --noPropertyAccessFromIndexSignature` reports errors only in c06 (index-sig/array element read), c07 (Record read) and c08 (`{a: undefined}`). tsc7 and tsrs both report 4 errors with the first two flags. The array-covariance write, width->optional re-add, casts, eval, Proxy, prototype mutation, enum and narrowing cases all still compile. `--erasableSyntaxOnly` additionally rejects only enums and parameter properties.
- [high] Engine-relevant TS-only semantics depend on tsconfig: the same class source gives different object shapes under useDefineForClassFields false vs true, and node's type stripping always uses [[Define]]. 4 of the 6 measured codebases compile with [[Set]] semantics (target<ES2022 default, or vscode's explicit false), and 50-63% of their class fields have no initializer.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/fields/f.ts:
- tsc --target es2020: `own keys of D instance: []` and `base setter ran with 1`, e.x=7
- tsc --target es2022 and `node f.ts` (strip-types): `["z"]`, e.x=1 own x
Tsconfigs: vscode src/tsconfig.base.json has useDefineForClassFields:false; the TS compiler, zod and pixijs target es2020. Fields without initializer: pixijs 893/1779, vscode 1192/1901 (results/summary.txt).
- [high] Real TS code is mostly 'typed', but the types carry little codegen information. Non-literal value expressions typed any/unknown/error are 0.2-5.6%, generic 0.5-5.2%, plain f64 'number' 3.5-25.3%. Function-typed expressions are 8.7-21.7% and lib objects (Map/Date/Promise/DOM) 1.2-10.4%.
  EVIDENCE: analyze.js (TS 6.0.3 checker, getTypeAtLocation on every non-literal value expression; assignments excluded; polymorphic `this` resolved to its class) over 6 codebases, all with 0-53 residual diagnostics:
- TypeScript v6.0.3 src/compiler: 148,724 code lines, 359,988 exprs
- zod 4.6.7 v4 core/classic/mini: 17,642
- date-fns core: 11,518
- hono 4.13.13: 19,458
- pixijs 8.22.0: 61,861
- vscode 01bfb51 base/common + editor/common: 85,516
Table in results/summary.txt.
- [high] Types alone prove essentially no int32 values. Of non-constant numeric expressions, only these shares have an int-literal-union or numeric-enum type: zod 2.9%, date-fns 8.5% (Day = 0..6 style unions), hono 2.6%, pixijs 1.0%, vscode 3.9%. The TS compiler is the outlier at 37.4%, from SyntaxKind/TypeFlags enums, which are unsound (any number is assignable to a numeric enum).
  EVIDENCE: Per-project aggregation of results/*.json exprKinds:
- typescript: 35,885 numeric non-const exprs, 13,434 int-typed
- zod: 1,199 / 35
- datefns: 2,798 / 237
- hono: 1,906 / 49
- pixijs: 26,378 / 273
- vscode: 39,987 / 1,571
Arithmetic/compare ops with BOTH operands int by type: 1.5-3.0% outside the TS compiler (TS compiler 56.6%, of which 6,772/9,672 are bitwise flag ops whose result is int32 by JS semantics anyway). The remaining 'int' hits are literal constants (`gl.TEXTURE_2D` is typed 3553 in lib.dom). c18 shows `const c: Color = n` compiles with n=42.
- [high] Receiver layout from types: only class-heavy code gives class receivers. Share of dynamic (non-namespace/static) property accesses whose receiver is an own class instance (incl. this): TS compiler 0.4%, zod 6.4%, date-fns 1.4%, hono 21.2%, pixijs 60.7%, vscode 59.2%. Interface/structural/union receivers are 59.1 / 55.4 / 44.1 / 26.6 / 18.5 / 16.8% of all a.b.
  EVIDENCE: results/*.json recvKinds.
The TS compiler declares 9 classes vs 1,201 interfaces+aliases; vscode 761 vs 928; pixijs 306 vs 562.
Even class types are structural in TS: `const fake: C = {x:1, m:()=>2}` compiles and `instanceof C` is false. `Object.create(C.prototype) as C` passes instanceof with fields missing (c24).
Only 3/9, 1/12, 2/35, 21/49, 188/306, 415/761 classes have private/protected/#x members (the nominal-ish ones); only hono uses #private (14 classes).
- [high] Escape hatches are common, and `any` contaminates more than its direct share: whatever value flows out of `any` or a cast is then trusted under a declared type.
  EVIDENCE: Per KLOC (code lines), order TS / zod / date-fns / hono / pixijs / vscode:
- `as T` (excl. as const, as any): 26.9 / 21.6 / 3.4 / 19.3 / 13.6 / 4.6
- `as any`: 0.54 / 20.6 / 0.43 / 3.7 / 1.5 / 0.78
- double `as unknown as T`: 0.24 / 5.5 / 0 / 1.6 / 0.8 / 0.5
- non-null `!`: 7.6 / 5.2 / 0.4 / 1.0 / 0.1 / 3.7
- `any` keyword in types: 1.4 / 34.2 / 0.8 / 22.4 / 10.1 / 3.9
- method signatures in types (bivariant): 10.1 / 11.7 / 0 / 1.8 / 2.1 / 10.3
- optional props declared: 7.3 / 20.5 / 5.2 / 44.3 / 13.4 / 16.8
pixijs compiles with strictNullChecks:false (pixijs/tsconfig.json), so null and undefined flow into every type there.
- [high] The TypeScript team itself had to restructure code for V8 runtime shapes; the static `Node` interface told the engine nothing. Optional properties and casts produced 'wrong map' deopts.
  EVIDENCE: microsoft/TypeScript PR #51682 'Reduce polymorphism resulting from unstable Node shapes' (merged 2022-12-13). It warns that optional properties, `node as any as JSDocContainer` casts and supertype properties 'often result in "wrong map" deoptimizations'. TS 5.0 release notes credit Node/Symbol monomorphization (with other changes) for builds at 80-90% of 4.9's time (devblogs.microsoft.com/typescript/announcing-typescript-5-0).
- [medium] In a real TS workload on V8, failed speculation is mostly shape-related, which TS types can't fix. Missing feedback, which ahead-of-time types could supply, is about a third. Int32 speculation almost never fails.
  EVIDENCE: `node --trace-deopt` of tsc 6.0.3 type-checking TypeScript src/compiler (node 24.22 / V8 13.6, about 4.1-4.4 s; tscheck/deopt.log): 1,434 eager + 259 lazy deopts.
- 'wrong map' 830
- 'Insufficient type feedback' for call 297, named access 146, binary op 83, compare 23, keyed 5 (554 total)
- dependent field-type constness changed 156
- prototype chain changed 80
- wrong call target 24
- 'not a Smi' 7
`--trace-opt`: 1,239 optimized compilations.
- [high] Making TS sound costs runtime checks or language restrictions. Fully typed overhead is single-digit to 15%. Mixed or deep checking can be catastrophic.
  EVIDENCE: Safe TypeScript (POPL'15, goto.ucsd.edu/~pvekris/docs/safets.pdf):
- fully typed Octane: 6.5% average slowdown; unannotated: 2.4x-72x (average 22x)
- bootstrapping its 90 KLOC compiler: 15% slowdown, and it found 478 static type errors
Takikawa et al. POPL'16 (Typed Racket): max overhead up to 121.5x and mean up to 53x on mixed configurations; fully typed vs untyped ratio 0.28x-13.34x.
Greenman & Felleisen ICFP'18: worst case up to 20.4x for first-order (transient) and 1072.8x for higher-order (deep) vs erasure ('10x' / '1000x').
Roberts et al. ECOOP'19: transient checks in a Truffle JIT average 5% (-13% to +79%).
Bauman et al. OOPSLA'17: Pycket's tracing JIT removes >90% of gradual-typing overhead.
Nom (OOPSLA'17): nominal-by-design, worst case <10%.
- [high] Sound typed-JS AOT systems all restrict the language and don't accept ordinary TS semantics.
  EVIDENCE: Static Hermes doc/TypedLanguage.md (facebook/hermes static_h, now the default branch; last change 2026-07-16) requires:
- exact objects ('must have exactly the set of properties... in the same order')
- nominal classes; arrays without holes; bounds-checked indexing ('throws on out-of-bounds')
- checked casts from any; `==` mostly disallowed
- getters/setters and overloads must be @Hermes.final
- no spread calls, async methods or computed class keys
It says 'typed language implementation is in progress and considered unstable'. Static Hermes RN-EU-2023 slides: 'The compiler cannot rely on type annotations'; nbody 5511 ms -> 565 ms typed native vs Hermes interpreter (about 9.8x); 'Without sound types... either a size explosion or interpreter-like performance'.
Static TypeScript (MPLR'19): nominal classes, casts checked (`(x as Class).field` costs 23 cycles vs `this.field` 6.5 on Cortex-M4), no dynamic property adds except map types. It is <2x slower than Node on property-heavy benchmarks and 20x slower on nbody (floating point).
- [high] Even sound types, used by a modified V8, gave little over V8's own speculation at peak.
  EVIDENCE: StrongScript (Richards, Zappa Nardelli, Vitek, ECOOP'15, Fig. 6), with unchecked intrinsics %_UnsafeAssumeMono/%_UnsafeAssumeFloat in an April 2015 V8:
- crypto +6.25%
- navier-stokes +20.59%
- raytrace +22.32%
- richards -0.86%
- splay +1.77%
The paper notes 'Many of [V8's] optimizations are redundant with our intrinsics'.
- [medium] The ceiling for removing guards in optimized JIT code is small, which bounds what type-based check elimination can buy at peak.
  EVIDENCE: Southern & Renau IISWC'16 (masc.soe.ucsc.edu/docs/iiswc16.pdf), V8/Octane: deopt-check branches are 6.2% of dynamic instructions; removing them gives 2.2% on a high-performance Intel core and 4.6% on a low-power one. An IISWC'21 abstract (computer.org CSDL 417300a013) reports about 8% average check overhead on JetStream2; I saw only the abstract via search, not the paper.
- [high] Engine teams that tried 'types for performance' in JS walked away. Global type inference cost more than it gave on real workloads.
  EVIDENCE: V8 strong mode / SoundScript, Rossberg 2016-02-11 (groups.google.com/g/strengthen-js/c/ojj3TDxbHpQ): 'our hope for being able to utilise strong mode for improving performance did not materialise'; 'we no longer believe that requiring type soundness by default can ever work successfully in JavaScript'; 'hundreds of new code paths'.
SpiderMonkey Warp (hacks.mozilla.org 2020/11): removing TI in favour of CacheIR feedback gave +10-12% Speedometer, +20% Google Docs load and -8% memory, but was slower on Octane/Kraken.
- [medium] Where types do pay off is AOT without a JIT or warm-up: sound types plus checks at boundaries.
  EVIDENCE: Dart ('Dart and the performance benefits of sound types'): Dart 1 types were 'effectively ignored'; AOT with speculation produced 'considerably worse code'; getAge went from 26 instructions (Dart 1 optimized) to 10 (Dart 2 AOT) to 3 (2.12 null safety).
mypyc docs: annotations enforced at runtime (TypeError); 1.5x-5x over CPython, mypy 4x.
Luau native codegen docs: argument annotations 'are checked'; a mismatch causes 'de-optimization or a fallback to interpreted execution' (types as AOT speculation).
Sorbet compiler: 22-170% over CRuby on Stripe traffic (2021); development paused, and the `compiler/` dir is gone from sorbet/sorbet (gh api 404, 2026-10-20).
Porffor (deda494, 2026-10-19) has `optTypes` on by default for .ts input: codegen.js:5428-5440 sets the function return type from the annotation, and coerceValue (codegen.js:74-88) converts rather than checks. Read only, not run.
- [medium] The type oracle itself is cheap enough to sit in an AOT pipeline.
  EVIDENCE: Type-checking TypeScript src/compiler (252 files, 264,187 lines incl. lib/@types), `--extendedDiagnostics` on this Mac (load average about 60):
- tsrs 0.18: check 0.113 s, total 0.138 s, 169 MB RSS; `--singleThreaded` 0.314 s, 130 MB
- tsc 7.0.2: check 0.464 s, 427 MB
- tsc 6.0.3: check 2.76 s, 717 MB
- [low] Warm-up, which ahead-of-time type seeding could attack, is a real but noisy share of short TS-tool runs.
  EVIDENCE: tscheck/warm.js runs 6 in-process iterations of createProgram+getPreEmitDiagnostics on TS src/compiler. Iteration 1 vs 6 ratio: 1.23x, 0.99x and 1.61x across runs (load average ~60, so low reliability). --max-opt=1 (Sparkplug cap) is about 3.5x slower and --max-opt=0 about 5x slower than default. Earlier execution-tiers research: TS workload Ignition 2.74 s, Sparkplug 1.75 s, Maglev 0.53 s, full 0.55 s.

## Risks
- The 'gain over a JIT' figures come from different eras and engines: StrongScript used 2015 V8/Crankshaft, Southern & Renau 2016 Octane, and I saw the IISWC'21 8% figure only in an abstract. Modern Turbofan/Maglev may have different check costs.
- Timing runs were on a heavily loaded machine (load average about 60, other agents running). Warm-up ratios and tsrs/tsgo/tsc check times are indicative only. Deopt counts are more robust but still vary run to run because of background compilation.
- The typedness metrics depend on my bucketing heuristics in analyze.js. Interface vs class uses the type symbol's flags. Lib detection uses declaration files. Assignment expressions and literal tokens are excluded. `this` is resolved through its base constraint. Expressions typed via `any` flows are counted under their declared type, so 'any' share understates real unsoundness.
- Residual diagnostics: pixijs has 53 (shader-file imports, GPU globals) and vscode 12 (2 external modules). These turn a small number of expressions into 'error' types.
- I built tsrs from the scratch clone (repos/tsrs at f527e984, same commit as third_party), not from third_party, with CARGO_TARGET_DIR in my work dir. tsrs reports TS 7.1.0-dev.20260929 semantics and could diverge from tsc 6/7 on edge cases. On this suite it matched: 0 errors under strict, 4 under the extra flags.
- Porffor's annotation trust comes from reading the source only. I did not execute it.
- Using TS hints as AOT speculation needs per-site deopt metadata. If ep9 deopts into the interpreter, wrong-typed calls that are frequent (e.g. code reached via `any`) could thrash. Exit counters and 'stop trusting hints at this site' are needed.

## Open questions
- How much do pre-seeded feedback vectors actually save on cold start (CLI, serverless) in a real engine? This needs an ep9 prototype or a V8 experiment that injects feedback.
- Should the sound subset follow Flow-like exact-object semantics (Static Hermes) or a TS-compatible nominal-class subset (STS/StrongScript)? This is a product decision about how much existing TS must compile unchanged.
- How should hints survive bundling and minification? They need a source-map-like side table keyed by oxc span, or a hint section in the bytecode cache.
- Are there cheap whole-program facts (no subclass, no delete/defineProperty on class X, no eval in module) that tsrs plus an engine-side linker could prove for typical app bundles? This needs measuring on bundled apps, not libraries.

## Artifacts
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/unsound/src (24 unsoundness cases c01..c24 + tsconfig.json)
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/unsound/run.log
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/analyze.js
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/summarize.js
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/results/summary.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/results/typescript.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/results/zod.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/results/datefns.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/results/hono.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/results/pixijs.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/results/vscode.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/fields/f.ts
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/tscheck/deopt.log
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/tscheck/warm.js
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/tscheck/tsconfig.ts-compiler.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/target-tsrs/release/tsrs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/ts-type-trust/papers (safets.txt, strongscript.txt, takikawa16.txt, greenman-icfp18.txt, roberts-ecoop19.txt, iiswc16.txt, nom.txt, sts-mplr19.txt, hermes-TypedLanguage.md, hermes-blog-*.md)
