# aot-perf-reality

## Summary
Typed AOT did not beat V8's top tier on any workload I measured.

**Builds and versions**
- Static Hermes: facebook/hermes branch static_h @5cf80e4e (2026-10-19). Release build with HERMESVM_ALLOW_JIT=2 and Apple clang 21. Configure took 17.5 s and ninja took 57 s.
- Porffor 0.0.0-alpha.18 (deda494, 2026-10-19). It now compiles JS to C and calls `cc -O3 -flto`.
- Others: AssemblyScript 0.28.20, wasmtime 49.0.3, node 24.22 (V8 13.6.233.17), bun 1.4.3 (JSC 0c06faad), QuickJS 2026-06-04, TypeScript 6.0.3 (runs on JS) and 7.0.2 (native Go).

**Method.** Each benchmark runs 5 rounds in one process. Numbers below are the best round in ms, over 3 processes. The ratio is against V8's best on equivalent source (×V8; above 1 means slower than V8).

**nbody** (2.5M steps per round; V8 = 90 ms):

| Engine / mode | ms | ×V8 | Note |
|---|---|---|---|
| C -O3 -mcpu=native | 52 | 0.58 | uses FMA, so not JS floating-point rules |
| C -O2 -ffp-contract=off | 74 | 0.82 | |
| AssemblyScript Wasm on JSC | 81 | 0.90 | |
| AssemblyScript Wasm on wasmtime | ~84 | 0.93 | |
| AssemblyScript Wasm on V8 | 99 | 1.10 | first round 245 |
| V8 full | 90 | 1.00 | |
| bun | 112 | 1.24 | |
| Static Hermes typed native | 173 | 1.92 | |
| V8 capped at Maglev | 246 | 2.73 | |
| Hermes JIT, typed | 354 | 3.93 | |
| V8, source with class field declarations | 517 | 5.74 | |
| Hermes JIT, untyped | 920 | 10.2 | |
| Static Hermes untyped native | 980 | 10.9 | |
| Porffor native | 1381 | 15.3 | wrong numeric result |
| Hermes interpreter | 1556 | 17.3 | |
| QuickJS | 4041 | 44.9 | |
| node --jitless | 4204 | 46.7 | |

**richards** (2000 iterations per round; V8 = 91 ms):

| Engine / mode | ms | ×V8 | Note |
|---|---|---|---|
| bun | 74 | 0.81 | |
| V8 full | 91 | 1.00 | |
| V8 capped at Maglev | 133 | 1.46 | |
| Static Hermes typed native | 288 | 3.16 | |
| Hermes JIT, untyped | 990 | 10.9 | |
| Hermes JIT, typed | 1114 | 12.2 | gets slower to ~1700 after round 1 |
| Static Hermes untyped native | 1158 | 12.7 | |
| Hermes interpreter, typed | 1485 | 16.3 | |
| Porffor | 1629 | 17.9 | |
| node --jitless | 2288 | 25.1 | |
| Hermes interpreter, untyped | 2357 | 25.9 | |
| QuickJS | 3422 | 37.6 | |

**raytracer** (3 frames of 256x256 per round; V8 = 96 ms):

| Engine / mode | ms | ×V8 | Note |
|---|---|---|---|
| bun | 77 | 0.80 | |
| V8 full | 96 | 1.00 | original TS-to-ES5 source: 213 |
| Static Hermes typed native | 148 | 1.54 | |
| V8 capped at Maglev | 152 | 1.58 | |
| Hermes JIT, typed | 275 | 2.86 | |
| Hermes interpreter, typed | 627 | 6.5 | |
| Hermes JIT, untyped | 937 | 9.8 | |
| Porffor | 1064 | 11.1 | the TS-to-ES5 version crashes |
| Static Hermes untyped native | 1186 | 12.4 | |
| Hermes interpreter, untyped | 1558 | 16.2 | |
| node --jitless | 1932 | 20.1 | |
| QuickJS | 2784 | 29.0 | |

**Octane, untyped JS** (geomean of 7 kernels; scores as a fraction of V8 full, so here above 1 means faster): V8 full 1.000, bun 1.041, Maglev cap 0.553, Hermes JIT 0.117, Static Hermes native 0.094, Hermes interpreter 0.069, node jitless 0.053, QuickJS 0.039. Porffor scores 0.078 on 6 kernels; it fails crypto.

**Answers**
- **Sound-typed AOT (Static Hermes typed native):** 1.5x to 3.2x slower than V8 full, geomean 2.1x. It is 1.4x faster than V8 capped at Maglev on nbody and ties Maglev on raytracer.
- **The best a fully typed AOT with no JS semantics can do:** C or Wasm on nbody runs 0.82x to 0.93x of V8's time, so only 1.1x to 1.2x faster. C reaches 1.7x faster only by using FMA, which JS floating-point rules forbid.
- **Unsound or untyped AOT:** never wins. Static Hermes untyped is 10x to 13x slower than V8, Porffor 11x to 18x. Both are only 1.3x to 2x faster than a good interpreter. Meta's own Oct 2026 numbers agree: native untyped tsc runs in 1.20 s against 354 ms on node.

**Warm-up: the best case AOT could win**
- Small kernels: V8 runs the first call about 20x slower than peak and reaches peak after about 2 to 3 ms. The total cost is 1.4 to 4 ms per process.
- tsc 6 on a 69k-line project: the first in-process run takes 1676 ms against about 1240 ms steady. Warm-up is 26% to 30% of a cold run, and the same holds on a small 0.36 s project.
- V8 spends 1.58 s of background compile CPU inside a 1.62 s run (3,608 Maglev and 884 TurboFan compiles).
- TypeScript 7, the native Go port, runs single-threaded in 0.45 s against 1.62 s for tsc 6 on V8: 3.6x less wall time and 6.5x less CPU.
- Startup: node 23.5 ms, bun 5.8, Hermes bytecode 4.0, Static Hermes native 4.9, QuickJS 2.0, Porffor 1.5.

## Recommendation
For ep9:

1. **Build the JIT path first: interpreter, then baseline, then a speculative mid tier.** This is the only path measured near V8. A Maglev-class tier alone gave 55% of V8 on Octane and matched V8 on tsc. No AOT system measured reached 50% of V8 except typed Static Hermes on floating-point code.

2. **Don't plan an AOT mode driven by TS types as the "faster than V8" tier.** Even a perfectly typed, non-JS AOT (C or Wasm) is only 1.1x to 1.25x faster than V8 on nbody. Sound typed AOT today is about 2.1x slower.

3. **Use TypeScript types inside the JIT, as speculation seeds that are guarded and can deopt.** TS types are unsound, so they cannot be trusted without checks. Good uses:
   - pre-shape objects and pick field representations (avoids V8's 5.7x class-field loss);
   - seed inline caches and feedback so the mid tier can compile at once (targets the 26% to 30% warm-up on tsc-sized runs);
   - skip type checks where the TS checker proves a type, with guards at module boundaries and on `any`.

4. **Make AOT a deployment mode, not a peak-speed tier.** Its gains are startup (1.5 to 5 ms vs node's 23 ms), memory (2 to 17 MB vs 50 to 60 MB), saving about 1x wall time in background compile CPU, and targets where JIT is forbidden (iOS). Concretely: snapshot or bytecode cache plus optional native code.

5. **If a typed AOT tier is ever built, it needs three things to compete:**
   - int32 or range inference, since TS `number` does not separate ints from doubles;
   - inline array access with hoisted bounds checks, unlimited inline slots, and no repeated checked casts;
   - JS floating-point rules kept exactly (no FMA contraction; Porffor gets this wrong).

   That is essentially the same SSA optimizer as the JIT mid and top tier, so share one optimizer between JIT and AOT. Feed it from runtime feedback in JIT mode and from TS types plus guards in AOT mode.

## Findings
- [high] Static Hermes builds from source on this Mac in about 75 s. The default branch is static_h; there is no separate shermes release and no current prebuilt binaries (the last GitHub release is v0.13.0 from 2024-08). One Release build gives hermes (interpreter plus arm64 JIT), hermesc and shermes. Native binaries link dynamically against libhermesvm.dylib (5.0 MB); -static-link failed with an undefined boost::context symbol.
  EVIDENCE: gh api repos/facebook/hermes: default_branch=static_h, pushed 2026-10-19; clone @5cf80e4e. `cmake -G Ninja -DCMAKE_BUILD_TYPE=Release -DHERMESVM_ALLOW_JIT=2` (17.5 s), then `cmake --build ... --target hermes hermesc shermes-dep -j14` (16:50:57 to 16:51:54). `hermes --version`: 'Hermes release version: 1.0.0, HBC bytecode version: 99, Features: JIT'. Link error: 'hoost::context::stack_traits::default_size()'. Logs: work/aot-vs-jit/hermes-build.log
- [high] Sound-typed AOT (shermes -typed, from the Flow-annotated typed sources that ship in Hermes's benchmarks/) is 1.5x to 3.2x slower than V8 full and never faster. Peak ms per round, typed AOT vs V8 best on the same algorithm: nbody 173 vs 90 (1.92x), richards 288 vs 91 (3.16x), raytracer 148 vs 96 (1.54x); geomean about 2.1x slower. Against bun (JSC) it is 1.5x to 3.9x slower. Hermes's own repo table agrees: richards typed native 3.4x slower than node.
  EVIDENCE: work/aot-vs-jit/results.json and results-table.md (3 processes x 5 rounds, min round). Commands: `shermes -O -typed X-typed.js -o bin`, `node X-nofields.p.js`. Checksums match V8 exactly (nbody -0.169193895234998, raytracer 576888). Repo table: repos/hermes/benchmarks/octane/richards-typed/benchmarks.md (node 19 ms vs typed native 65 ms at 400 iterations, 2026-04-24).
- [high] Typed AOT beats V8's mid tier on floating-point code but not on object-heavy code. Against node --max-opt=2 (Maglev cap): nbody 173 vs 246 ms (AOT 1.42x faster), raytracer 148 vs 152 (tie), richards 288 vs 133 (AOT 2.2x slower).
  EVIDENCE: results-table.md, rows node-maglev(max-opt=2) vs shermes-native typed
- [high] Type annotations give 4x to 8x inside Static Hermes's native path, about 2.6x to 3.4x in its JIT on nbody and raytracer, and 1x to 2.5x in its interpreter. Untyped vs typed native: nbody 980 vs 173 (5.7x), richards 1158 vs 288 (4.0x), raytracer 1186 vs 148 (8.0x). Hermes JIT: nbody 920 vs 354, raytracer 937 vs 275; on richards the typed JIT run is slower (1114 ms in round 1, then about 1700), an unexplained anomaly. Interpreter: nbody 1.0x, richards 1.59x, raytracer 2.5x.
  EVIDENCE: results-table.md, rows hermes-interp, hermes-jit and shermes-native in typed and untyped variants; hbc made with `hermesc -O [-typed] -emit-binary`, JIT run with `hermes -Xjit file.hbc`
- [high] Code with full static types and no JS semantics is only 1.1x to 1.25x faster than V8 full on nbody, V8's best case. C with clang -O2 -ffp-contract=off gives 74 ms per round and bit-identical output, against V8's 90. AssemblyScript to Wasm gives 81 ms on JSC and about 84 ms on wasmtime/Cranelift, but 99 to 103 ms on V8 (first round 245, while Liftoff tiers up). C -O3 -mcpu=native gives 52 ms (1.73x faster) only by using FMA, which changes the result (-0.169194062057166), so JS cannot legally do it.
  EVIDENCE: work/aot-vs-jit/bound.log and c/nbody.c, as/nbody.ts, as/run.mjs. Commands: `clang -O2 -ffp-contract=off`, `asc nbody.ts -O3 --runtime stub --noAssert`, `wasmtime compile` then `run --allow-precompiled --invoke nbody nbody.cwasm 12500000` (0.42 s for 12.5M steps).
- [medium] Untyped AOT is 10x to 18x slower than V8 full and only 1.3x to 2x faster than interpreters. It is also slower than Hermes's own arm64 JIT on untyped code. Static Hermes untyped vs V8: nbody 10.9x, richards 12.7x, raytracer 12.4x slower. Porffor: 15.3x, 17.9x and 11.1x. Octane geomean as a fraction of V8 full: Static Hermes native 0.094, Hermes JIT 0.117, Hermes interpreter 0.069, Porffor 0.078 (6 kernels), node jitless 0.053, QuickJS 0.039.
  EVIDENCE: results-table.md (high confidence) and work/aot-vs-jit/octane-table.md (max of 2 reps; per-kernel scores in octane-results.json). Octane bundles reused from work/exec-pipeline/bench, built with `shermes -O`, `hermesc -O -emit-binary` and `porf native`. Octane is noisy: node-full richards varied 31013 to 68401 between reps.
- [medium] Meta's own Oct 2026 data shows the same result on a real program. tsc built as an untyped Static Hermes native binary takes 1.20 s, against 1.27 s as Hermes bytecode and 354 ms on node (V8). The native binary starts in 18 ms against node's 75 ms. The talk's summary slide reads: 'AOT native ... best for startup, shipping, typed code, Wasm; JIT best for hot, dynamic, untyped code'.
  EVIDENCE: T. Mikov, 'Compiling a real-world CLI tool to native with Static Hermes', next.app devCon 2026 Berlin, 2026-10-09: https://speakerdeck.com/tmikov2023/compiling-a-real-world-cli-tool-to-native-with-static-hermes (slides 35 and 39, read with WebFetch; slide text came through an extraction tool).
- [high] Why typed Static Hermes still lags V8: every `number` is a NaN-boxed double. There are no native ints, so loop indices are doubles and bit operations convert back and forth. Each typed-array element read is an out-of-line call (`_sh_fastarray_load`, which takes a double index). Objects have only 5 inline property slots, and a 6th field costs a runtime call. Checked casts and use-before-init checks repeat on field reads, and a virtual call returns `any`. V8 instead learns int32 and shape facts from runtime feedback.
  EVIDENCE: repos/hermes/include/hermes/VM/static_h.h:1650-1652 (`SHERMES_EXPORT SHLegacyValue _sh_fastarray_load(SHRuntime*, SHLegacyValue*, double index)`); emitted C in work/aot-vs-jit/emit/nbody-typed.c around line 303 (double loop counters, `_sh_prload` and `_sh_prstore_number`); repos/hermes/benchmarks/octane/richards-typed/optimization-notes.md ('Static Hermes does not yet support native integer types'; 5 direct slots; `__sh_prload_indirect`). Untyped C (emit/nbody-untyped.c) does a generic lookup of `Math` and then `sqrt` plus `_sh_ljs_call` on every iteration.
- [high] Static Hermes's typed mode does not take real TypeScript. With `-typed -parse-ts`, even a 2-line file fails because the internal TypedLib does not parse ('TypedLib:36:6: error: '(' expected in method definition'). The ts2flow converter also rejects interfaces, type literals, generic type parameters, unions and `as`. With the Flow parser, only TS that is also valid Flow compiles (nbody.ts works). Parameter properties and string-literal unions fail. The typed dialect is sound and Flow-like: exact objects, nominal classes, bounds-checked arrays, checked casts from `any`. It does not yet support spread arguments, rest destructuring, async or generator methods, computed class keys, or optional calls. `--transform-ts` cannot be combined with `-typed`.
  EVIDENCE: work/aot-vs-jit/tstest/*.ts probe output; repos/hermes/doc/TypedLanguage.md:311 ('Not supported (yet)'); doc/typescript-stripping.md ('--transform-ts is incompatible with --typed')
- [high] V8 runs ES2022/TS class field declarations badly on numeric fields, which is relevant to TS-emitted code. The class-based nbody with `x; y; ...` declarations (what flow-remove-types or TS useDefineForClassFields emit) runs 517 ms per round on V8, against 90 ms without the declarations: 5.7x slower. JSC is unaffected (112 vs 114). The cause: the field starts as `undefined`, so its representation is generalized to Tagged and every double store allocates a new HeapNumber. A TS annotation `x: number` would let an engine pick the double representation up front.
  EVIDENCE: results-table.md, node-full stripped vs nofields. `node --trace-generalization -e "class A { x; constructor(x){this.x=x} } new A(1.5)"` prints `[generalizing]x:h{Any;const}->t{Any;mutable} (uninitialized field)`; the same class without `x;` prints no generalization.
- [high] Porffor's C backend breaks JS floating-point rules. clang contracts a*b+c into an FMA, so nbody prints -0.16919383316763523 where every other engine prints -0.169193895234998. Rebuilding with `CC="cc -ffp-contract=off"` restores exact output. Porffor also fails Octane crypto ('Crypto: ERROR [object Object]') and the TS-emitted ES5 raytracer ('Uncaught TypeError: Cannot set property of undefined'). Its TS-type optimization (optTypes, on by default) made no measurable difference on nbody.ts (1455 to 1562 vs 1545 to 1618 ms per round). It self-reports test262 at 92.05% (49157/53404) as of 2026-10-19. Static Hermes avoids the FMA problem because it emits one operation per C statement.
  EVIDENCE: work/aot-vs-jit/porfcheck/t.js (outputs differ: `node -e` shows 0.09376554183952797 !== 0.093765541839528); repos/CanadaHonk_porffor/compiler/index.js (cc -O3, flto); repos/CanadaHonk_porffor/test262/history.json, latest entry
- [medium] On small kernels, V8 warms up in about 2 to 3 ms, so removing warm-up saves only 1.4 to 4 ms per process. V8's first call costs 0.75 to 0.83 ms, about 20x its peak of 0.036 to 0.046 ms. It is within 20% of peak after about 10 to 20 calls and at peak by about 50. The total excess over peak is 1.4 to 4 ms; JSC's is 1.4 to 7 ms. Break-even with typed AOT: Static Hermes typed beats V8 only while the hot code's total runtime is under about 3 ms. Richards break-even is at about 15 to 20 calls; nbody at about 45 chunks of 1000 steps.
  EVIDENCE: work/aot-vs-jit/warm.log, from scripts/genwarm.py per-call drivers using performance.now; real node 24.22 and bun.native binaries
- [medium] On a short compiler run, warm-up is 26% to 30% of wall time. This is the upper bound on AOT's warm-up gain, and only if AOT code matched V8 peak. Measured with tsc 6.0.3 on engine262/src (69k TS lines plus 14k library lines, 1.6 s).
- In-process iteration 1 takes 1676 ms against 1194 to 1284 ms once steady.
- Small project (278 lines plus 55k DOM library lines): 330 vs 223 to 240 ms.
- Wall time by tier cap: Ignition only 7.92 s, Sparkplug cap 4.19 s, Maglev cap 1.53 s, full 1.62 s. The mid tier alone already gets the full result.
  EVIDENCE: work/aot-vs-jit/scripts/tsc-repeat.cjs (10 fresh createProgram + getPreEmitDiagnostics per process); warmup-cli-tsproj.json and warmup-cli-tsproj-small.json (best of 3)
- [high] The JIT's hidden cost is CPU. During one 1.62 s tsc run, V8 compiled 2,121 distinct functions: 3,608 Maglev compiles (518 ms) and 884 TurboFan compiles (1,058 ms). That is 1.58 s of compile CPU, almost all on background threads; main-thread finalize is about 14 ms. The run also had 1,548 deopts (725 'wrong map'). User CPU is 3.38 s against 1.62 s wall. With --single-threaded the wall time doubles to 3.19 s. AOT removes this CPU but would barely change wall time when spare cores exist.
  EVIDENCE: `node --trace-opt --trace-deopt tsc.js -p tsconfig.json`, parsed into work/aot-vs-jit/prof/trace-opt.txt; /usr/bin/time -p from warmup.py. The counts are high-confidence; the CPU-ms sums are V8's own timers. (`--prof` tier attribution failed on macOS: 64% of ticks were wrongly attributed to node::JSONWriter.)
- [medium] Rewriting the compiler in a statically typed AOT language (TypeScript 7, the native Go port) gives 3.6x lower wall time and 6.5x less CPU than tsc 6 on V8, single-threaded on the same project. tsc7 --singleThreaded takes 0.45 s (user 0.52 s) against 1.62 s (user 3.38 s); multi-threaded it takes 0.21 s. On the small project: 0.15 vs 0.36 s. This is not the same algorithm: tsc7 reports 72k types against tsc6's 42k, and both report the same 15 diagnostics.
  EVIDENCE: warmup-cli-tsproj.json; `tsc --extendedDiagnostics` outputs; npm typescript@7.0.2 ships per-platform native binaries
- [high] AOT clearly wins on startup and memory. Hello-world median (hyperfine, 40 runs): Porffor native 1.47 ms (2.2 MB RSS), QuickJS 1.99 ms (2.7 MB), Hermes bytecode 4.03 ms (8.5 MB), Static Hermes native 4.87 ms (9.8 MB; dynamic libhermesvm), bun 5.77 ms (10 MB), node 23.5 ms (46 MB; another +5 ms through the vite-plus shim). Peak RSS in the benchmarks: node 50 to 62 MB, bun 18 to 49 MB, Static Hermes 10 to 17 MB, Porffor 2 to 38 MB.
  EVIDENCE: work/aot-vs-jit/startup.json; RSS from `/usr/bin/time -l` in results.json
- [high] Compile cost of AOT. shermes takes 0.13 to 0.42 s for the kernels and 5.0 s for earley-boyer (208 KB of source), against 0.03 s for hermesc bytecode. Porffor takes 1.1 to 1.9 s for the kernels and 37.5 s for earley-boyer. Binaries are 4x to 8x larger than bytecode: earley-boyer is 950 KB native vs 200 KB .hbc (Porffor 1.6 MB). Meta's talk reports a 26.3 MB to 69.8 MB size increase for native tsc and builds of about 53 s.
  EVIDENCE: /usr/bin/time -p shermes/hermesc runs; `ls -la work/aot-vs-jit/octane/bin`; speakerdeck slide 35

## Risks
- The machine was shared: load average 9 to 17 on 18 cores, from other agents. Long single-threaded runs varied up to 2x between rounds; one node --jitless nbody round took 11.4 s against 4.4 s. I report the minimum round and best total over 3 processes. Octane node-full scores varied by up to 2.2x between reps (I take the max of 2). Expect ±10-15% on microbenchmark cells and more on Octane.
- The typed sources are the Hermes authors' rewrites. They differ from the originals: inlined helper functions, a Ray class instead of object literals, ES classes. For fairness V8 was given the same restructured code with types stripped. On the original raytracer source V8 is 2.2x slower (213 vs 96 ms), so the 'V8 best' reference depends on how the source is written.
- All of these are small kernels plus one compiler workload. Real typed applications were not measured with typed AOT; the only real-app AOT number is Meta's tsc (untyped), read from slides via WebFetch.
- Hermes typed-JIT richards gets slower after round 1 (1114 ms, then about 1700). I did not investigate, so the Hermes JIT numbers may understate it.
- `node --prof` tier attribution was unusable on macOS because symbolization put 64% of ticks on node::JSONWriter. Tier timing therefore rests on the tier-cap runs, in-process iteration curves and --trace-opt compile times.
- TS7 (Go) vs TS6 (JS) is a different implementation (72k vs 42k types), not a same-algorithm AOT comparison.
- C/AS nbody uses arrays of pointers to heap structs to mirror JS objects. Struct-of-arrays C would be faster still, but that is not JS-shaped code.
- `node` and `bun` on PATH are vite-plus shims that add about 5 ms to startup. Benchmark tables use in-process timers, so they are unaffected; startup numbers use the real binaries.

## Open questions
- Would Static Hermes typed code close the gap to V8 with native int types, inline array loads and more inline slots? Its own notes list these as missing. No such build exists to measure.
- How fast would a JIT seeded with TypeScript types be (TS types as speculation, deopt when violated)? No existing engine does this, so it cannot be measured; it would need an ep9 prototype. The target is the 26-30% warm-up on short compiler runs and the field-representation losses.
- Typed AOT on large real typed apps: Meta's ditz2 went from 940 to 505 ms bytecode-to-native, but there is no V8 comparison for it.
- Do the per-round variations come from P-core vs E-core scheduling? macOS has no CPU pinning; a quiet machine is needed for tighter error bars.
- How does V8 handle class fields across versions (node 24 / V8 13.6 measured)? Does V8 main (15.x) still generalize `x;` fields to Tagged?

## Artifacts
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/results-table.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/results.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/matrix.log
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/octane-table.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/octane-results.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/bound.log
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/warm.log
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/warmup-cli-tsproj.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/warmup-cli-tsproj-small.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/prof/trace-opt.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/startup.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/emit/nbody-typed.c
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/emit/nbody-untyped.c
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/scripts/gen.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/scripts/matrix.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/scripts/octane.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/scripts/warmup.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/scripts/tsc-repeat.cjs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/scripts/genwarm.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/scripts/bound.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/bench
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/c/nbody.c
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/as/nbody.ts
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/tstest
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/aot-vs-jit/hermes-build/bin
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/hermes
