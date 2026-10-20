# js-aot-precedents

## Summary
I looked at 18 precedents. On this Mac (2026-10-20) I built and ran Static Hermes (static_h 5cf80e4), Porffor (ba4813d/deda494), scriptc 0.3.6, node 24.22 (V8), bun 1.4.3 (JSC) and QuickJS 2026-06-04.

**Result:** no precedent beats a top-tier JIT across the board.
- Typed AOT wins in only one case: numeric code over fixed-layout objects with unboxed doubles. scriptc ran nbody about 1.3x faster than V8.
- It also wins on startup and memory.
- It lands near the mid tier (Maglev class) on OO code like Richards, and loses badly on untyped code.
- AOT without types lands at baseline-JIT speed, about 2-4x QuickJS.

SPEED LADDER (QuickJS = 1x; my runs; richards x4000 / nbody 5M)
```
scriptc TS-typed     ███████████████████████████ 28x / ████████████████████████████████████████████████████ 54x  <- beats V8 on nbody only
V8 TurboFan          ██████████████████████████████████████ 37-39x / ███████████████████████████████████████ 38-43x
JSC FTL              ███████████████████████████████████████████ 44x / ████████████████████████████████████ 36x
V8 Maglev only       ███████████████████████ 23-27x / ██████████ 10-13x
StaticHermes typed   ████████████ 12x / ██████████████████████ 22x
Hermes JIT (untyped) ████ 3.5-4.6x / ████ 4.5x
SH untyped native    ███ 3.0-3.6x / ████ 3.7x      <- untyped AOT ~= baseline JIT
Porffor native       ██ 1.9-2.0x / ███ 3.1x
V8 jitless           █ 1.4-1.5x / █ 0.97x
scriptc UNTYPED JS   0.35x / 0.2x                   <- types-or-nothing cliff
```

PRECEDENTS TABLE

| precedent | types | language scope | output | status (Oct 2026) | compatibility | performance |
|---|---|---|---|---|---|---|
| Static Hermes typed | Sound. Flow syntax; TS partly via ts2flow. Checked casts, exact objects, nominal classes | Typed subset with changed semantics | C/LLVM native, Wasm, typed bytecode | static_h is the default branch. Typed/native mode is not shipped in React Native | Hermes is at 41.8% on test262.fyi | Measured: richards 585ms vs V8 182-192ms, nbody 375ms vs 196-223ms. Meta's own run: 65ms vs 19ms on Node |
| Static Hermes untyped native | Inference only | Full Hermes JS | Native binary | hermes-node demo, 2026-10 | As above | Measured: same speed as its baseline JIT. Talk: tsc 1.20s vs Node 354ms |
| Hermes bytecode | None | Full | .hbc (mmap) | Default in React Native. Hermes V1 is the default engine from RN 0.84 | As above | Measured on a 9.1MB file: 14.9ms vs 836ms from source |
| Porffor | Trusts TS types without checks (unsound) | Aims for full JS | JS to C to native | Alpha, one author, rewritten 2026-07 | 92.05% on its own runner, 91.17% on test262.fyi | Measured: 19x slower than V8 on richards, 12x on nbody. About 2-3x QuickJS |
| Hopc | None; inference plus optimistic guards | ES5.1 plus most of ES2017 | Scheme, then C | Research | Not reported | ICFP 2021: many JetStream tests 1-2x slower than V8. Weak on floating-point and call-heavy code. At least 3x less memory |
| Static TypeScript (STS) | Nominal static subset | Subset (no eval, prototypes, arguments) | ARM Thumb | Used in MakeCode | Not applicable | vs Node 11: richards 1.8x slower, nbody 20x slower |
| SJS (Samsung) | Sound, inferred | Fixed-layout subset | C | Dead | Not applicable | 1.5-9.8x slower than V8 on long runs |
| AssemblyScript | Strict TS-like types | Not JS semantics | Wasm | Active | Not JS | Its own FAQ: "not always" faster |
| V8 strong mode / SoundScript | Sound gradual types (planned) | Opt-in VM mode | Inside V8 | Abandoned 2016-02 | Not applicable | No performance result |
| TC39 Type Annotations | Ignored by engines | Not applicable | Not applicable | Stage 1, last presented 2023-09 | Not applicable | Performance is an explicit non-goal |
| scriptc (Vercel Labs) | TS7-checked, runtime-checked casts | Subset; refuses eval; quickjs-ng fallback | LLVM native, Wasm | Experimental 0.3.6 | No full test262 | Measured: nbody 157ms (beats V8), richards 259ms (about Maglev speed). Untyped: 20.6s and 41s |
| Perry | Not enforced | Prototype edits are ignored | SWC + LLVM | 0.5.x | Claims about 97% of Node's own tests | Self-reported, mixed results |
| weval / StarlingMonkey | None | Full SpiderMonkey | Wasm AOT | Active | Full | 2.77x over its interpreter; SpiderMonkey's native baseline JIT is about 5x |
| qjsc / Javy / LLRT | None | QuickJS | Bytecode | Active / experimental | QuickJS-family | Startup gains only. Measured qjsc: 37ms vs 380ms |
| Bun --bytecode, V8 code cache | None | Full | Engine bytecode cache | Shipping | Full | Measured startup about 2x faster |
| Dart 2 (non-JS) | Moved to sound types to make AOT effective | Not applicable | Not applicable | Not applicable | Not applicable | Not applicable |
| Luau | Annotations used as hints with slow fallback | Not applicable | Not applicable | Not applicable | Not applicable | Not applicable |

## Recommendation
LESSONS FOR ep9 (each backed by the findings above):

1. Do not promise that AOT beats V8 in general.
   - Typed AOT has beaten TurboFan/FTL in only one shape of code: numeric code over fixed-layout objects with unboxed f64 fields. scriptc ran nbody 1.3x faster than V8.
   - On OO and dispatch-heavy code it reaches about Maglev level (scriptc Richards) or 2-3x slower than V8 (Static Hermes, STS, SJS, Hopc).
   - The reliable AOT wins are startup (2-50x) and memory (3-10x).

2. Types pay off only if they are sound and change the data layout. Without types, AOT is baseline-JIT class (2-4x QuickJS: Static Hermes untyped, Porffor, weval). So the ep9 AOT tier must turn tsc/tsrs types into:
   - fixed object layouts with unboxed f64/int32 fields (avoid Hermes' NaN-boxed fields and its 5-slot inline limit);
   - direct calls and devirtualization;
   - fewer checks, via narrowing across field reads.

3. TS types are unsound, and the language is spec-bound. ep9 targets FULL ECMAScript, and test262 forbids the semantic changes Static Hermes and scriptc make (throw on out-of-bounds or on a cast). Porffor shows that trusting types without checks gives wrong answers (17 instead of 'a1'). So in default mode, ep9 should treat TS types as SPECULATION, not truth:
   - Compile AOT code from types plus guards: a shape check, or a tag check on entry and at field load.
   - Deoptimize into the same interpreter/JIT frames on a mismatch.
   - Share one IR and deopt metadata between the AOT tier and the mid-tier JIT.
   - This is Hopc's 'optimistic compilation', with TS types replacing inference.
   - An opt-in sound 'strict typed module' mode (Static Hermes style, checked at module boundaries) can come later, behind a flag, and outside test262.

4. Use TS types in the JIT too. This is cheap and needs no AOT. Example: seed field representations and hidden-class layouts from class field types. V8 loses 6-7x on TS ES2022 `x: number;` fields; JSC does not.

5. Build the interpreter/JIT and AOT bytecode first. Every full-language AOT keeps an interpreter for eval/new Function and untyped npm code (Amdahl's law). The cheapest big win is a serializable, mmap-able bytecode and snapshot format: Hermes .hbc took 14.9ms vs 836ms from source on 9MB of TypeScript, and qjsc 37ms vs 380ms.

6. Native-backend correctness rules:
   - Compile with -ffp-contract=off (or emit no FMA); Porffor's nbody result differs without it.
   - Keep exact IEEE and ToInt32 semantics.
   - Watch compile time and binary size: shermes tsc grew 26 -> 70MB, and a build took about 2 minutes.

7. Pipeline:
```
TS/JS --tsrs checker (types as hints)--> ep9 IR --> [AOT: guarded typed code + bytecode]
                                                  `-> [interp -> baseline -> mid-tier JIT]   (shared deopt)
```

## Findings
- [high] Measured: Static Hermes sound-typed native AOT is 3.1-3.2x slower than V8 full on Richards and 1.7-1.9x slower on nbody. It is 2x slower than V8 Maglev-only on Richards but 1.8-2.2x faster than Maglev on nbody, and 12x/22x faster than QuickJS. Typed mode helps mostly in the native backend. Typed vs untyped on the same algorithm: native 2020->585ms (3.5x), JIT 1566->1488ms (1.05x), interpreter 3873->2928ms (1.3x).
  EVIDENCE: Static Hermes was built from facebook/hermes static_h 5cf80e4 (2026-10-19) as Release with HERMESVM_ALLOW_JIT=1, using shermes -O [-typed]. Benchmarks are the Hermes repo's own richards.js and richards-typed.js (x4000 iterations) and nbody original vs fully-typed (n=5M). Each number is the best of 3. Machine load average was 10-24.

Richards (ms):
- V8 full: 182-192
- JSC: 164-173
- V8 --max-opt=2 (Maglev): 263-308
- V8 --max-opt=1 (Sparkplug): 2682
- V8 jitless: 4709-4983
- QuickJS: 7121-7174
- Static Hermes typed native: 585
- Static Hermes untyped native: 2020-2391
- Hermes JIT, untyped / typed: 1566-2047 / 1488

nbody (ms):
- V8 full: 196-223
- JSC: 231-237
- Maglev: 662-843
- QuickJS: 8435
- Static Hermes typed native: 375
- Static Hermes untyped native: 2238-2258
- Hermes JIT typed: 705-798

Result files: scratchpad/work/aot-precedents/richards-results.txt and scratchpad/work/aot-precedents/nbody-results.txt.

Meta's own table (2026-04-24, Richards x400) agrees: typed native 65ms vs Node 19ms (3.4x), 13x faster than QuickJS. Source: scratchpad/repos/facebook_hermes/benchmarks/octane/richards-typed/benchmarks.md
- [high] Why Static Hermes typed code stays slower than V8:
- `number` is still a NaN-boxed double, and there is no native int type. Bitwise ops cost fjcvtzs/scvtf round trips.
- Objects have 5 direct slots. Field 6 and later goes through a runtime call (__sh_prload_indirect); moving one hot field saved 8%.
- There is no narrowing across this.field reads, so about 20 redundant CheckedTypeCasts remain per hot method.
- Calls through `any` return need casts.
- Little inlining.
  EVIDENCE: scratchpad/repos/facebook_hermes/benchmarks/octane/richards-typed/optimization-notes.md (sections 'Numbers are NaN-boxed doubles', 'Property access: direct slots vs indirect', 'Redundant CheckedTypeCastInst'; result 72ms->66ms)
- [high] Static Hermes typed mode is a different language from TS/JS, not 'JS plus types':
- Flow syntax; TS only partially, via -parse-ts converted to Flow.
- Exact objects with a fixed property order; nominal classes.
- Arrays cannot have holes, and out-of-bounds access throws RangeError.
- `==` is mostly disallowed. Getters/setters must be @Hermes.final.
- Unannotated functions are not type-checked.
- Not supported: spread call arguments, rest destructuring, async/generator methods, computed class keys, optional calls.
- Soundness is enforced at runtime. Each of these throws instead of following JS behavior: an `any` flowing into a typed variable, JSON.parse into `number`, writing a typed field through `any`, and reading arr[5] on a 3-element typed array.
  EVIDENCE: scratchpad/repos/facebook_hermes/doc/TypedLanguage.md.

Experiment: scratchpad/work/aot-precedents/sound/lie.flow.js, run with `shermes -typed -exec` and `hermes -typed`. Output:
- 'caught: TypeError: Checked cast failed' (twice)
- 'TypeError: Cannot assign to read-only property x'
- 'RangeError: array load index out of range'

In plain JS these would print 'a1', '71', NaN and undefined. The TS version failed with: 'ts2flow: remaining TS node TSAsExpression'.
- [high] Static Hermes status and adoption (Oct 2026):
- static_h is the default branch of facebook/hermes.
- React Native 0.84 (2026-02-11) made 'Hermes V1' the default engine. That is the static_h compiler+VM running untyped bytecode. RN announces no typed or native AOT.
- Meta's 2025 Hermes v2 numbers (hv32+JIT, a baseline-style JIT) are only about 2-5.6x QuickJS on Octane, and the QuickJS used was the pre-2026 version.
- 2026-10-09 talk (hermes-node), untyped native: tsc ran 1.20s vs 1.27s as bytecode and 354ms on Node V8.
- Native startup was 18ms vs Node 75ms, but the talk credits caching, not native code.
- Binaries grew: tsc 26.3 -> 69.8MB. ditz2 needed about 103s to compile.
- AOT cannot cover eval/new Function, so the JIT or interpreter handles them.
  EVIDENCE: Repository: gh api repos/facebook/hermes reports default_branch=static_h.

RN 0.84 blog: https://reactnative.dev/blog/2026/02/11/react-native-0.84

Hermes blog posts:
- scratchpad/repos/facebook_hermes/doc/blog/2025-07-15-static-h-performance-june-2025.md (charts read from images/2025-07-15-perf-hv32-jit.jpeg)
- scratchpad/repos/facebook_hermes/doc/blog/2025-11-02-hermes-compilation-runtime-modes.md
- scratchpad/repos/facebook_hermes/doc/blog/2024-12-19-static-hermes-update-dec-2024.md (the JIT gains on InterpDispatch2 (7.9x) and Mandelbrot (3.29x) come from types the compiler inferred)

Talks:
- https://speakerdeck.com/tmikov2023/compiling-a-real-world-cli-tool-to-native-with-static-hermes
- RN EU 2023 deck: nbody 5511ms (interpreter) -> 565ms (typed native)
- React Universe 2024 deck: 'Moderate speed ups for untyped code', and Amdahl's law cited as the limit set by untyped code.

test262.fyi Hermes 41.80%: scratchpad/research/rust-engine-landscape.md:58
- [high] The strongest typed-AOT result I found is scriptc. It uses the TypeScript 7 checker and LLVM, gives classes exact C-struct layouts with unboxed f64 fields, and uses refcounting plus cycle collection.
- nbody: 157-166ms, 1.25-1.4x faster than V8 (196-223) and JSC (231-237), and bit-identical output.
- Richards: 259-334ms, about V8 Maglev speed (263-308) and 1.4x slower than TurboFan.
- The same compiler on plain untyped JS: Richards 20,599-21,435ms (107x slower than V8, 2.9x slower than QuickJS) and nbody 41,136ms (5x slower than QuickJS). Its coverage report still said '100% compile statically' for both.
- Class-field JS with no annotations compiled (98% static) but then threw at runtime: 'TypeError: expected null | undefined ... got object'. That is valid JS rejected.
  EVIDENCE: npm scriptc 0.3.6 (2026-10-17). Install scripts were blocked by npm, so I ran @scriptc/cli-darwin-arm64 after chmod +x, with PATH=/usr/bin first for Apple clang.

Results: scratchpad/work/aot-precedents/scriptc-results.txt

The LLVM IR shows `%sc_o__x25_cx311_Body = type { i64, double x7, ptr }`: scratchpad/work/aot-precedents/scriptc/nbody-typed.ll line 24.

Docs: https://scriptc.dev/docs/limitations
- refuses eval/new Function, prototype reflection, and object-vs-primitive ==
- out-of-bounds reads on typed arrays terminate the process
- `--dynamic` falls back to an embedded quickjs-ng

tests/test262/README.md in vercel-labs/scriptc says its pass counts 'must not be presented as whole-suite ECMAScript conformance'.
- [high] Trusting TS annotations without checks (unsound) produces wrong results. Porffor applies TS annotations by default (optTypes is on by default) and does not check them.
  EVIDENCE: scratchpad/work/aot-precedents/sound/lie.ts

Code tested: function add1(x:number){return x+1}; add1("a" as any), and let n:number = JSON.parse('"7"'); n+1.
- node strip-types prints 'a1' and '71'.
- Porffor (both `porf lie.ts` and native) prints 17 and 16657.

Source: scratchpad/work/aot-precedents/porffor/compiler/prefs.js line 1 (optTypes on by default), and codegen.js:2576-2660 (extractTypeAnnotation, promotes locals to raw f64).

Perry docs also say 'declared types aren't enforced at runtime' and that prototype edits don't change class layout: https://docs.perryts.com/language/limitations.html
- [high] Porffor (JS to C to native AOT; types not used for speed, measured):
- Richards 3539-3729ms: 19x slower than V8, 1.9-2.0x faster than QuickJS.
- nbody 2658-2752ms: 12x slower than V8, 3.1x faster than QuickJS.
- TS annotations gave no speedup on nbody (2658 vs 2666ms).
- Values are 16-byte structs {f64 val; i32 type}.
- Own test262 runner: 92.05% (49157/53404) on 2026-10-19, up from 69.5% on 2026-10-10. test262.fyi: 91.17%.
- Its site claims a V8-v7 suite score of 6646, vs 5828 for Node --jitless and 3564 for QuickJS, and says it is 'not at JIT level yet'. Node/Bun with JITs score 13-17x higher.
  EVIDENCE: scratchpad/work/aot-precedents/porffor-results.txt

Generated C header: 'typedef struct jsval { f64 val; i32 type; } jsval;' in scratchpad/work/aot-precedents/bench/nbody.porf.c

test262 history: scratchpad/work/aot-precedents/porffor/test262/history.json

Site claims: https://porffor.dev/
- [high] Compiling through C/LLVM changes JS floating-point results unless FMA contraction is turned off. Porffor's clang -O3 build on arm64 prints nbody energy -0.16908313397886804. V8, JSC, shermes and scriptc all print -0.16908313397889935. Recompiling the same Porffor C with -ffp-contract=off gives the V8 value.
  EVIDENCE: scratchpad/work/aot-precedents/bench/nbody.porf.c

I compiled it with `clang -O3 -ffp-contract=off` and with `clang -O3`. Porffor's own flags (compiler/index.js:128-135) have no -ffp-contract.
- [high] AOT without types lands at baseline-JIT speed:
- shermes untyped native: 3.0-3.7x QuickJS, about the same as Hermes' own baseline JIT (3.5-4.6x).
- Porffor: 2-3x QuickJS.
- weval AOT of SpiderMonkey (interpreter partially evaluated against 2,367 precompiled IC stubs): Octane geomean 2.77x over SpiderMonkey's interpreter (Richards 4.39x, CodeLoad 0.90x). SpiderMonkey's native baseline JIT gets about 5x.
- Hermes' Mikov: untyped tsc native gave only -6% vs bytecode.
  EVIDENCE: Measured files above. weval: https://cfallin.org/blog/2024/08/27/aot-js/ (table, plus 'native baseline compiler ... around a 5x geomean') and https://cfallin.org/blog/2024/08/28/weval/
- [high] Hopc (Serrano, Inria) is the closest full-language AOT precedent.
- Covers all of ES5.1 plus most of ES2017.
- No type annotations: it uses type and range inference plus 'optimistic compilation' with guards, and does hidden classes and inline caches statically.
- Pipeline: JS to Scheme to C.
- ICFP 2021: on JetStream2 'For many tests, Hopc performance is in the range 1x..2x when compared to V8'. It is weak on floating-point code (crypto, n-body, raytrace) because doubles are boxed (almabench about 3x slower), and on deltablue because it does not inline.
- eval needs an interpreter fallback.
- 'Hopc uses at least 3x less memory than V8'.
- The paper's conclusion: whether AOT can match JITs 'is still unanswered'.
- DLS 2018 abstract: the majority of benchmarks are within 50% of one of the fastest JITs.
- ECOOP 2022, sealed classes: +19% average, +56% best case.
- 2024/25 Float Self-Tagging paper (Melançon, Serrano, Feeley) targets the boxed-float weakness.
  EVIDENCE: scratchpad/work/aot-precedents/docs/icfp21.txt, lines 1365-1408, 1619-1620, 2111-2125 (from serrano-icfp21.pdf, doi 10.1145/3473575; tested against V8 6.8, JSC 4.0, SpiderMonkey 68).

https://conf.researchr.org/event/dls-2018/dls-2018-javascript-aot-compilation
https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ECOOP.2022.24
https://arxiv.org/abs/2411.16544

manuel-serrano/hop was last pushed 2026-02-16.
- [high] Static TypeScript (MakeCode, MPLR 2019) is a sound-ish nominal subset compiled to ARM Thumb.
- It drops with, eval, prototype inheritance, `this` outside classes, `arguments` and `apply`.
- Numbers are 31-bit tagged ints, and other numbers are boxed doubles.
- On a Pi Zero, measured as times slower than GCC:
  - richards(100k): Node 9x, STS 16x (STS 1.8x slower than V8)
  - fann(9): 3x vs 20x (6.7x slower)
  - nbody(100k): 4x vs 81x (20x slower)
  - binary(11): 1.7x vs 2.2x
- It is more than 10x faster than Duktape, IoT.js and MicroPython, and runs in 16KB of RAM.
  EVIDENCE: scratchpad/work/aot-precedents/docs/sts.txt, lines 215-240 and 676-716 (Figure 4), from https://www.microsoft.com/en-us/research/uploads/prod/2019/09/static-typescript-draft2.pdf. microsoft/pxt-common-packages was last pushed 2026-10-12.
- [high] SJS (Samsung, OOPSLA 2016): sound inferred types plus fixed object layout, compiled to C. It is competitive with V8 only at small iteration counts. At the largest runs it is 1.5x (navier) to 9.8x (raytrace) slower than V8, while using much less memory. For example, raytrace x1000 took 19.6s vs 2.0s on V8, and richards x10000 took 3.42s vs 1.48s.
  EVIDENCE: scratchpad/work/aot-precedents/docs/sjs.txt, lines 1056-1075 and Figure 27 (lines 1885-1925), from https://arxiv.org/pdf/1608.07261
- [high] V8 strong mode / SoundScript (2015-2016):
- Plan: a gradual, sound, TypeScript-based type system implemented inside the VM. Its 'opportunities' slide lists 'More optimisations! aggressive ones require soundness', 'No opt/deopt cycles of death', 'Reduced warm-up time' and 'Ahead-of-time compilation/optimisation'.
- Its challenges slide lists 'Must not break lazy compilation ... precludes non-local type inference'.
- 2016-02-11: Rossberg posted that V8 'reluctantly decided not to pursue strong mode further'. Reasons: class sealing fell apart under interop with ordinary 'weak' classes, 'hundreds of new code paths' in V8, and poor ES6 performance. The team no longer believed soundness could work 'by default' in JS, though opt-in might.
- The code was removed from V8.
  EVIDENCE: scratchpad/work/aot-precedents/docs/tc39-2015-012.txt (from Ecma/TC39/2015/012, lines 196-221 and 393-412)

https://groups.google.com/g/strengthen-js/c/ojj3TDxbHpQ/m/T6bTRDlnEwAJ
https://2015.ecoop.org/details/STOP2015/3/Experiments-with-Strengthening-JavaScript
https://codereview.chromium.org/1773653002 ('[strong] Remove all remainders of strong mode')
- [high] The TC39 Type Annotations proposal is still Stage 1. The tc39/proposals table lists notes up to 2023-09, and the repo was last pushed 2025-08. The README says 'At runtime, a JavaScript engine ignores them, treating the types as comments'. Its FAQ says the authors are 'not aware of successful experiments in JavaScript that meaningfully beat dynamic type-driven JIT optimization', and that improving performance is 'explicitly a non-goal'. It cites TS* for runtime-check slowdown, and calls strong mode 'canceled ... in failure'.
  EVIDENCE: scratchpad/work/aot-precedents/docs/type-annotations-README.md, lines 6, 12, 719-736 and 783.

gh api repos/tc39/proposals/contents/stage-1-proposals.md (Type Annotations row)
- [high] Measured startup on TypeScript 5.9.3 lib/typescript.js (9.1MB), loading it plus one transpileModule call (hyperfine -N, 10 runs, mean, busy machine).

AOT bytecode:
- Hermes .hbc: 14.9ms, 22.9MiB, vs Hermes source with -lazy at 836ms. Building the .hbc with hermesc -O took 8.2s and produced 6.48MB.
- qjsc bytecode-embedded exe: 37ms, 24.6MiB, vs qjs source at 380ms.

Caches:
- bun --bytecode: 77.5ms vs 165ms. The .jsc file is 10.35MB, more than the 8.46MB JS.
- Node NODE_COMPILE_CACHE warm: 88ms vs 176ms (2.5MB cache).

Empty-script floors: node 37ms, bun 32ms, qjs 2.7ms, hermes 5.7ms.
  EVIDENCE: scratchpad/work/aot-precedents/startup-results.txt

Workload: scratchpad/work/aot-precedents/startup/ts-load.js

Bun docs claim 2x startup for tsc: https://bun.com/docs/bundler/bytecode. The V8 code cache blog reports a 20-40% compile-time cut: https://v8.dev/blog/improved-code-caching. Node's compile cache dropped its experimental status in v25.4: https://nodejs.org/api/module.html#module-compile-cache
- [medium] Hermes' original reason for AOT bytecode (2019) was build-time compile, mmap'd bytecode and no JIT. Mattermost on Android: TTI 4.30s -> 2.29s, download size 41-42MB -> 19-22MB, memory 185 -> 136MB. On iOS, Callstack reports about 40% faster start than JSC. LLRT (QuickJS, no JIT, still 'experimental') claims up to 10x faster startup and 2x lower cost on Lambda, but says it is worse on long loops. Javy builds 1-16KB Wasm modules with dynamic linking, vs 869KB static.
  EVIDENCE: https://engineering.fb.com/2019/07/12/android/hermes/ (the figures are in images; the numbers come from secondary sources found by search: next.ink and the Mattermost Chain React 2019 blog)

LLRT: gh api repos/awslabs/llrt/readme, lines 5-10 and 428-438.
Javy: gh api repos/bytecodealliance/javy/readme, lines 20-22.
- [high] TS type info could help even a JIT. TypeScript with target >= ES2022, and Node's stripTypeScriptTypes, turn `x: number;` into a class field `x;` initialized to undefined. On V8 that made nbody 6-7x slower: 1184-1428ms vs 196-201ms without the fields, and 196-199ms with `x = 0.5`. JSC was unaffected (231-237ms). Likely mechanism: the field representation starts as a heap object, never becomes Double, and every store boxes a HeapNumber. A TS-aware engine could seed the field representation from the declared type.
  EVIDENCE: scratchpad/work/aot-precedents/nbody-results.txt (last lines)

TS emit check: ts.transpileModule on TS 5.9.3 shows ES2020 dropping the field, while ES2022 and ESNext emit 'x;'. node -e module.stripTypeScriptTypes keeps 'x        ;'.

The mechanism is inferred, not traced with --trace-generalization.
- [medium] Other 2024-2026 TS-to-native compilers:
- Perry (PerryTS/perry): created 2026-01, about 5k stars, SWC + LLVM, Rust. Self-reported results are mixed: wins on matrix_multiply (19ms vs Node 33) and json_roundtrip (144 vs 379); losses on method_calls (35 vs Node 11 / Bun 8) and binary_trees (13 vs 6). Types are not enforced, and weak references are never collected.
- ts-aot (OffByOneStudios): 19 stars.
- ASDAlexander77/TypeScriptCompiler (LLVM/MLIR, since 2021): 776 stars.
- Third-party scriptc reports: startup about 1.8-12.5ms vs Node 62-68ms; memory 2-8MB vs 70-86MB. Hono needed --dynamic, which put 62% of the code in quickjs-ng and gave 18.4k vs Bun 70.5k req/s. One Hacker News user found a byte-array workload 7.5x slower than Node 24.
  EVIDENCE: scratchpad/work/aot-precedents/docs/perry-README.md, lines 34-72

https://docs.perryts.com/language/limitations.html

gh api repos/OffByOneStudios/ts-aot and repos/ASDAlexander77/TypeScriptCompiler

scriptc third-party numbers come from search summaries of infoq.com/news/2026/09/vercel-scriptc-node, developersdigest.tech and daily.dev. I did not verify them.
- [medium] AssemblyScript is not JS semantics:
- 'there is no any or undefined' and 'no union types yet'; objects must be typed as a class or Map.
- 'it is unlikely that existing TypeScript code can be compiled'.
- Its FAQ: 'pre-existing TypeScript code doesn't magically become faster just by compiling to WebAssembly'. It is faster only for low-level numeric code.
- Still active: v0.28.20, 2026-07-22.
  EVIDENCE: https://www.assemblyscript.org/concepts.html
https://www.assemblyscript.org/frequently-asked-questions.html
gh api repos/AssemblyScript/assemblyscript/releases
- [medium] Typed precedents outside JS point the same way:
- Dart moved from unsound optional types (Dart 1) to sound types (Dart 2), largely so AOT would work. Its docs: 'While AOT compilation is possible without types, the generated code is much less efficient'. Soundness relies on runtime checks for casts from dynamic and for covariant parameters.
- Luau's native codegen uses annotations only as hints. A wrong type takes a slower fallback path rather than producing wrong results. Reported speedup is about 1.5-2.5x on compute code.
- Safe TypeScript measured about 15% overhead for sound runtime checks on its own roughly 90k-line compiler.
  EVIDENCE: https://dart.dev/language/type-system
https://dart.cn/blog/dart-and-the-performance-benefits-of-sound-types
https://news.dartlang.org/2016/11/dart-strong-mode-and-ahead-of-time_30.html
https://create.roblox.com/docs/luau/native-code-gen
https://goto.ucsd.edu/~pvekris/docs/safets.pdf (POPL 2015)
- [high] Every working full-language AOT system keeps an interpreter or JIT next to the AOT code, so in practice they are already mixed-mode systems:
- Static Hermes: native, bytecode, lazy source and JIT can mix in one runtime, and an embedded interpreter handles eval/new Function.
- scriptc: static code plus a quickjs-ng fallback.
- Hopc: eval runs in the Bigloo interpreter.
- weval: the interpreter stays the fallback.
  EVIDENCE: scratchpad/repos/facebook_hermes/doc/blog/2025-11-02-hermes-compilation-runtime-modes.md
scratchpad/repos/facebook_hermes/doc/blog/2024-12-23-compiling-javascript-to-wasm.md, line 35

scriptc limitations page; icfp21.txt lines 1394-1400.

## Risks
- Only two small kernels were measured (Richards, nbody). Both are monomorphic and favor typed AOT. No large-app result exists for any typed AOT, so do not extrapolate.
- The machine was shared, with load average 10-30 on 18 cores. Individual cells may be off by 10-20%. I report best of 3, and hyperfine means with high sigma.
- My Static Hermes typed interpreter and JIT numbers are 1.5-1.7x worse than Meta's own table, though typed native matches. Meta probably used a different build config (HV32, LTO). I did not verify this.
- I made richards-typed.ts with a sed rewrite of the Flow file (?T to T | null). It typechecked and ran under scriptc, but it is not an official TS port.
- scriptc's native binary was run by chmod +x on the npm platform package, because npm blocked install scripts. Its toolchain wrote .scriptc-native-* temp dirs under my work dir. Porffor wrote a build cache to ~/.cache/porffor/build.
- I ran `git fetch` once inside the shared Porffor clone at scratchpad/repos/CanadaHonk_porffor. Only refs changed; HEAD was already at deda494.
- These figures are secondary or self-reported and I did not verify them: scriptc third-party numbers, Perry's tables, the Mattermost 2019 figures, the KusStar Static Hermes numbers, and Luau's 1.5-2.5x.
- The V8 class-field slowdown mechanism (field representation stuck at Tagged) is inferred, not traced.

## Open questions
- How much real-world TS is 'sound enough' to compile typed? Run the tsrs checker over popular npm packages and the ep9 target apps, and measure the share of values typed any or unknown, and of cast or assertion sites.
- Can speculative typed AOT with deopt reach mid-tier JIT quality on polymorphic OO code (deltablue, TypeScript compiler)? No precedent tested this; Hopc comes closest, without types.
- Can TS-type hints measurably cut JIT warmup (pre-seeded ICs, field representations, inlining decisions) on Octane or Web Tooling? Only the single class-field case is shown here.
- Is Hopc competitive on 2026 hardware against current V8? Its only numbers are from 2021 against V8 6.8, and I did not build Bigloo/Hop.
- Was the Sorbet compiler (typed Ruby AOT) abandoned? Its compiler/ directory returns 404 on sorbet/sorbet master, but I found no announcement. Low priority.
- What do test262 pass rates look like for typed-mode subsets? None of Static Hermes typed, scriptc, Perry or AssemblyScript publish a whole-suite number.

## Artifacts
- scratchpad/work/aot-precedents/richards-results.txt
- scratchpad/work/aot-precedents/nbody-results.txt
- scratchpad/work/aot-precedents/porffor-results.txt
- scratchpad/work/aot-precedents/scriptc-results.txt
- scratchpad/work/aot-precedents/startup-results.txt
- scratchpad/work/aot-precedents/run-bench.sh
- scratchpad/work/aot-precedents/bench
- scratchpad/work/aot-precedents/sound/lie.ts
- scratchpad/work/aot-precedents/sound/lie.flow.js
- scratchpad/work/aot-precedents/scriptc/nbody-typed.ll
- scratchpad/work/aot-precedents/hermes-build/bin/shermes
- scratchpad/work/aot-precedents/docs/icfp21.txt
- scratchpad/work/aot-precedents/docs/sts.txt
- scratchpad/work/aot-precedents/docs/sjs.txt
- scratchpad/work/aot-precedents/docs/tc39-2015-012.txt
- scratchpad/work/aot-precedents/docs/type-annotations-README.md
- scratchpad/work/aot-precedents/docs/perry-README.md
- scratchpad/repos/facebook_hermes/doc/TypedLanguage.md
- scratchpad/repos/facebook_hermes/benchmarks/octane/richards-typed/benchmarks.md
- scratchpad/repos/facebook_hermes/benchmarks/octane/richards-typed/optimization-notes.md
