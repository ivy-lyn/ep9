# execution-tiers

## Summary
I measured 13 engine/tier configs on this machine (M-series, node 24.22 / V8 13.6, bun 1.4.3 / JSC, QuickJS 2026-06-04, Boa, Brimstone, Nova), and the data gives a clear tier picture. On 7 Octane kernels (geomean, QuickJS = 1.0, best of 3): good interpreters land at 1.2-1.5x (Brimstone in Rust 1.23, V8 Ignition 1.33-1.36, JSC LLInt 1.48). Baseline JITs land at 2.1x (V8 Sparkplug) and 3.9x (JSC Baseline). Speculative mid-tier JITs land at about 14x (Maglev 13.98, DFG 13.72), which is 54% of V8 full (25.6x). On a short TypeScript-compiler workload, the mid tier alone already matches full V8/JSC (0.53s vs 0.55s; 0.51s vs 0.50s). So the speculative SSA mid tier with deopt is the tier that matters for "production + speed". The interpreter caps out at roughly 1.5x QuickJS no matter how it is written. Rust explicit tail calls (`become`) are still nightly-only and experimental: tracking issue rust-lang/rust#112788 is OPEN with the B-experimental label, stable 1.99 rejects them, and the project goal aims to stabilize in 2027. On nightly they compile to true threaded code (`br x0` / `jmpq *`), with args pinned to x20/x21 (r12/r13 on x86) under `extern "rust-preserve-none"`. That is 1.3-1.5x faster than a match loop on a dispatch-bound microbenchmark and within about 10% of C computed goto. Stable Rust gets the same machine code by relying on LLVM sibling-call optimization, but nothing guarantees it: the debug build overflowed the stack. Cranelift 0.136 compiled baseline-shaped code at 11-21 µs per bytecode. That is about 2,600-5,000x slower than raw template emission (4 ns per bytecode), and Cranelift has no deopt stackmaps. So it does not fit a baseline tier and is a weak fit for the mid tier, but it is a possible backend for a later top tier. V8 status (main, v15.8, 2026-10-20): Turboshaft has replaced Sea of Nodes in the JS backend, and `turbolev` (Maglev frontend feeding Turboshaft) still defaults to false at flag-definitions.h:1808. JSC is building a new register-allocating baseline JIT called "LOL" (default off). On Apple Silicon, MAP_JIT plus a per-thread W^X toggle works for an unsigned or adhoc-signed Rust binary, and a toggle pair costs about 23 ns. That favors data-driven ICs and indirect code-entry tables over patching code.

## Recommendation
STAGED PLAN. Each stage reuses the previous stage's data structures; nothing gets thrown away.

```
S0 specs ──> S1 Rust interp+ICs ──> S2 JIT infra ──> S3 baseline JIT ──> S4 speculative mid-tier ──> S5 top tier (optional)
             (~1.3-1.5x QJS)                          (2-4x QJS)          (~14x QJS, ≈55% V8 Octane,     (~25x QJS)
                                                                            ≈100% V8 on short tooling)
```

**S0. Specs.** Fix the five hard-to-change decisions listed below, write them down, and put each one behind a single Rust module. Also stand up a differential test harness (ep9 vs node/QuickJS) and the test262 runner.

**S1. Rust interpreter.**
- Register bytecode with an accumulator.
- Feedback vector and data-driven ICs from day one. Brimstone shows a Rust match-loop VM with NaN-boxing, shapes and ICs already reaches 1.23x QuickJS.
- Write each handler once through a macro DSL that expands to three builds:
  - (a) `loop { match }`: debug builds and fallback;
  - (b) stable release using sibling calls (`return TABLE[op](..)`), guarded in CI by a disassembly or stack-depth check, the way Pulley does it;
  - (c) nightly `become` + `extern "rust-preserve-none"` as an opt-in build. Do not make the whole workspace depend on nightly: `become` is not expected to stabilize before 2027, and tsrs pins stable 1.99.1.
- Pin pc, frame base, accumulator and the dispatch table in handler arguments.
- Keep fast paths tiny and put slow paths in `#[cold] #[inline(never)]` Rust functions.
- Target: V8 Ignition or JSC LLInt level, 1.3-1.5x QuickJS.

**S2. JIT infrastructure.**
- A MacroAssembler for aarch64 and x64. dynasm-rs is fine as the encoding layer.
- Executable memory: on macOS, MAP_JIT + pthread_jit_write_protect_np + sys_icache_invalidate (all verified). On Linux, dual-mapping or mprotect.
- Frame walking and unwinding for JS exceptions, GC stack maps or root scanning for JIT frames, and perf jitdump.
- Per-function code-entry slot or dispatch table, so tier-up swaps a pointer and never patches call sites (V8 JSDispatchTable).

**S3. Baseline template JIT.**
- Start Sparkplug-style: one pass over bytecode, the interpreter's frame layout, shared feedback and ICs, calls to shared stubs. Expect about 1.5x over the interpreter.
- Then inline IC and int32 fast paths the way JSC Baseline does. Expect about 2.6x over the interpreter (3.9x QuickJS on Octane) and 3.1x on the TypeScript workload.
- OSR interpreter→baseline at loop headers: copy the frame.

**S4. Speculative mid tier (Maglev/DFG class). This is the minimum tier for "production + speed".**
- Its own CFG SSA IR, not Sea of Nodes (V8 lesson), built from bytecode plus a feedback snapshot taken on the main thread and compiled off-thread (Warp style; fits Rust ownership well).
- Optimizations: representation selection (unboxed int32 and f64), shape guard + direct slot load/store, elements-kind specialization, small inlining, a linear-scan register allocator, and its own backend on the S2 assembler.
- Deopt to interpreter or baseline frames from per-exit metadata (bytecode offset plus where each live register and the accumulator are). Use lazy reconstruction (JSC MovHint style) or V8-style frame-state translations.
- Per-bytecode exit counters, so a failed speculation is not repeated.
- OSR entry at loop headers.
- Code dependencies (watchpoints/fuses) with lazy deopt for on-stack frames.
- Target, measured on peers: about 10x over the interpreter and about 14x QuickJS (54% of V8 full) on Octane, and about equal to full V8/JSC on short compiler-like workloads.

**S5. Top tier (later).** Two options:
- (i) Extend our own IR with Turboshaft-class passes: LICM, load elimination, escape analysis, range analysis, loop peeling.
- (ii) Lower the S4 IR to Cranelift, with deopt exits as cold calls whose arguments are all live values. Cranelift has no ValueRep-style stackmaps. Its measured 11-21 µs per bytecode is fine only for hot code.
- Expect about 1.8-2.0x over the mid tier (V8 TurboFan/Maglev 1.83x, JSC FTL/DFG 2.0x).
- Do not use LLVM (the JSC FTL lesson).

**S1.5 (optional, once S3 exists).** Generate a "baseline interpreter" from the S3 code generator, as SpiderMonkey and JSC LLInt do: native-stack frames identical to JIT frames, and it can be embedded at build time for a JIT-free mode, as V8 embedded builtins are. Keep the Rust interpreter as the portable and reference tier.

**HARD-TO-CHANGE DECISIONS (fix in S0).**

1. **Value representation.** Use 64-bit NaN-boxing with JSC encoding: raw pointers in 0000:xxxx, doubles offset by +2^49, int32 under the FFFE tag.
   - Pros: no HeapNumber allocation, so no need for V8's field-representation and mutable-heap-number machinery. Pointer loads need no decode. Same family as JSC, SpiderMonkey, Brimstone and Boa. Raw pointers keep conservative stack scanning possible (GC coupling).
   - Cons: 8-byte slots (V8 pointer compression saves up to 43% heap), and heap addresses must fit in 47-48 bits. Guard this with mmap hints or reserved regions; Boa issue #4275 and LA57 show the risk.
   - The JIT bakes the encoding into every tag check, so decide before S3.
2. **Bytecode format.**
   - Register machine + accumulator (about 46% fewer dispatches than a stack VM).
   - 1-byte opcode, operands normally 1 byte, wide/extra-wide prefixes (as Ignition, JSC and Brimstone do).
   - Explicit feedback-slot operands.
   - Relative jumps.
   - Side tables for exception handlers, liveness (needed by deopt/OSR) and lazily built source positions.
   - Generator suspend/resume as bytecodes.
   - Bytecode offset is the single deopt and OSR target identity.
3. **Frame layout.**
   - One canonical JS frame: caller FP, return address, callee, argc, CodeBlock/bytecode, feedback vector, environment, bytecode-offset slot, then the register file.
   - Arguments pushed in reverse, with missing ones padded with undefined (V8 adaptor-frame lesson: +11-40% on calls).
   - The interpreter's VM stack, baseline frames and deopt-materialized frames all use this layout, so OSR and deopt are copies.
   - Decide early whether interpreter frames live on a separate VM stack (Rust interpreter) or the native stack (generated interpreter); S1.5 is the bridge.
4. **IC and feedback design.**
   - Per-function FeedbackVector, allocated lazily (V8 waits 8 invocations).
   - Slot kinds: property load/store, keyed access, call (target + count), binary/compare type lattice, elements kind / allocation site, for-in, plus exit flags.
   - IC entries are data: CacheIR-like guard+action lists with stub fields, monomorphic up to about 4 polymorphic entries, then a megamorphic global stub cache.
   - The interpreter interprets them, baseline compiles them, and the mid tier transpiles them.
   - No code patching, which stays W^X-friendly (toggle pair about 23 ns) and jitless-friendly.
5. **Object and string model.**
   - Shapes hold prototype, elements kind, transition tree and flags, with in-object slots plus out-of-line slots and slack tracking; dictionary mode on churn.
   - Elements are separate, with V8-style kinds (PACKED/HOLEY × SMI/DOUBLE/ELEMENTS, one-way).
   - Strings: Latin1 or UTF-16 flat, ropes, slices (threshold about 13), internalized atoms with cached hash, and external/Rust-owned. Never use Rust UTF-8 `String` as the JS string representation.

**PERFORMANCE MILESTONES.** Octane geomean on M-series, QuickJS = 1.0, my measurements:

| Milestone | Octane | TypeScript cold run |
|---|---|---|
| S1 interpreter | 1.2-1.5 (V8 jitless 1.33, LLInt 1.48; about 5% of V8 full) | 2.7-3.3s (QuickJS 4.3s) |
| S3 simple baseline | about 2.1 | about 1.75s |
| S3 JSC-quality baseline | about 3.9 | about 1.06s |
| S4 mid tier | about 14 (54% of V8 full 25.6) | about 0.5s (= V8 full) |
| S5 top tier | about 25-28 | |

Boa (0.19) and Nova (0.08) show how badly a naive Rust VM design can miss these numbers.

## Findings
- [high] Rust explicit tail calls (`become`, feature explicit_tail_calls) are NOT stable as of Oct 2026. The tracking issue is OPEN with label B-experimental ('RFC pending'). Its remaining work includes 'Handle indirect ABI arguments' and 'wait for good LLVM support of all supported targets'. Stable rustc 1.99.0 rejects it with E0554.
  EVIDENCE: gh issue view 112788 -R rust-lang/rust (state OPEN, updated 2026-07-12, labels F-explicit_tail_calls, B-experimental); local: `cargo +1.99.0 build --features tail` -> error[E0554]: #![feature] may not be used on the stable release channel (/private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/src/main.rs:1)
- [high] Expected stabilization timing: the 2026 project goal says 'not quite aiming for stabilization this year... hopefully stabilize in 2027'. Remaining blockers are wasm (needs the tail-call feature), PowerPC, and an MVP limited to scalar or register-passed arguments.
  EVIDENCE: https://trifectatech.org/blog/tail-calls-project-goal/ (Folkert de Vries, 2026-04-14)
- [high] Related nightly ABIs exist. `extern "rust-preserve-none"` (feature rust_preserve_none_cc, merged PR #151065 Jan 2026, tracking #151401 OPEN) and `extern "tail"` (tracking #157427 OPEN). LLVM preserve_nonecc is supported only on x86_64, AArch64 and LoongArch.
  EVIDENCE: gh issue view 151401 / 157427 -R rust-lang/rust; gh pr 151065 merged 2026-01-25; https://llvm.org/docs/LangRef.html (preserve_nonecc: 'Currently it is for x86_64, AArch64 and LoongArch only')
- [high] On nightly-2026-10-04 (rustc 1.101), `become` through a static handler table compiles to perfect threaded code. Each handler ends in `ldr x0,[table,op,lsl#3]; br x0` on aarch64. With rust-preserve-none, VM state stays pinned in x20/x21. Cross-compiled x86-64 output is `jmpq *(%rcx,%rax,8)` with state in r12/r13. A full Add handler with overflow check is 13 instructions.
  EVIDENCE: objdump of /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/dispatch-nightly symbol tail13preserve_none3add; rustc --target x86_64-unknown-linux-gnu --emit asm -> /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/x86.s
- [medium] Dispatch microbenchmark: identical register bytecode, 1e9 dispatches, 10 ops per loop. The machine was loaded (load average about 11 on 18 cores). Times are ns per dispatch, best/typical: match loop 0.69-0.80; loop-call through a fn-pointer table (Boa's style) 0.91-1.0; stable sibling-call 0.53-0.60; nightly become 0.52-0.59; become with preserve_none 0.51-0.62; C computed goto (clang 23) 0.48-0.50. Tail-call dispatch is about 1.3-1.5x faster than match and within about 10% of C.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/src/main.rs:46 (match), :85 (loopcall), :98 (become), :133 (sibcall); /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/cgoto.c; hyperfine: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/hyperfine-dispatch.md (match 850±28ms, loopcall 979, sibcall 603, tail 587, tail-pn 617, cgoto 499)
- [high] On stable Rust, `return TABLE[op](..)` in release mode compiles to the same `br x3` as `become`. It is not guaranteed, though: the debug build of the same code aborted with 'thread main has overflowed its stack', while the nightly `become` debug build ran fine. Wasmtime's Pulley interpreter handles this with cfg pulley_assume_llvm_makes_tail_calls (unverified TCO) or pulley_tail_calls (nightly become). Pulley defaults to its match loop even though its own docs call the tail loop 'more performant'.
  EVIDENCE: ./target/debug/dispatch sibcall 1000000 -> 'fatal runtime error: stack overflow'; /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/wasmtime/pulley/src/interp/tail_loop.rs:1-27 and pulley/src/interp.rs:20-23
- [medium] Third-party data on what tail-call dispatch is worth: Wasmi CoreMark scores are 2820 (direct tail-call), 2357 (indirect tail-call), 1764 (loop-match) and 1133 (loop-call). A specialized bytecode VM got 7.9% faster with become + rust-preserve-none and had 47% fewer branch misses. CPython 3.14's tail-call interpreter (Clang 19+, x86-64/AArch64) reports 3-5% on pyperformance. Gains shrink as handlers get bigger.
  EVIDENCE: comments by Robbepop and coastalwhite on https://github.com/rust-lang/rust/issues/112788; https://docs.python.org/3.14/whatsnew/3.14.html ('A new type of interpreter')
- [medium] Octane steady state, 7 kernels (richards, deltablue, crypto, raytrace, earley-boyer, splay, navier-stokes; regexp excluded because Nova fails it). Geomean relative to QuickJS (=1.00), best of 3: Boa 0.19, Nova 0.08, Brimstone 1.23, V8 jitless 1.33, V8 Ignition (max-opt=0) 1.36, V8 +Sparkplug (max-opt=1) 2.08, V8 +Maglev (max-opt=2) 13.98, V8 full 25.62, JSC LLInt (useJIT=0) 1.48, JSC +Baseline (no DFG) 3.90, JSC +DFG (no FTL) 13.72, JSC full 27.72.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/scripts/run_matrix.py, /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/octane-run.log, /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/octane-table.md, scripts/summarize.py output
- [medium] Step factors between tiers in that data. V8: Sparkplug/Ignition 1.53x, Maglev/Sparkplug 6.7x, TurboFan/Maglev 1.83x. JSC: Baseline/LLInt 2.63x, DFG/Baseline 3.5x, FTL/DFG 2.0x. The mid tier (Maglev or DFG) alone reaches 54-55% of full V8 on Octane. JSC Baseline produces about 1.9x faster code than Sparkplug. My inference: JSC Baseline inlines IC and arithmetic fast paths, while Sparkplug mostly calls builtins.
  EVIDENCE: derived from octane-run.log; Sparkplug design quote 'most Sparkplug code just calls into builtins' https://v8.dev/blog/sparkplug; JSC Baseline = 'bytecode template JIT' https://webkit.org/blog/10308/speculation-in-javascriptcore/
- [medium] On a short compiler-like workload (Octane TypeScript: 2.5 MB source, 5 iterations, no warmup; wall time, best of 3), the mid tier already equals full peak. Results: QuickJS 4.30s, Brimstone 3.59s, V8 jitless 2.80s, Ignition 2.74s, +Sparkplug 1.75s, +Maglev 0.53s, V8 full 0.55s; JSC LLInt 3.32s, +Baseline 1.06s, +DFG 0.51s, full 0.50s. Boa failed with 'RangeError: bad length for array'. Nova failed with 'backreferences are not supported' in its regex.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/scripts/run_ts.py, /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/ts-run.log
- [high] Survey of Rust JS engines. Brimstone uses an Ignition-inspired register bytecode with a match-loop dispatch, NaN-boxing with a Smi tag, shapes plus inline caches, and a compacting GC; it is already faster than QuickJS (1.23x). Boa uses a loop over a 256-entry fn-pointer table (OPCODE_HANDLERS), register operands and NaN-boxed JsValue, and runs at 0.19x QuickJS. Nova uses a stack-based VM with a data-oriented SoA heap and runs at 0.08x. None of the Rust engines I checked has a JIT.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/brimstone/README.md; brimstone/src/js/runtime/bytecode/vm.rs:770-1050; brimstone/src/js/runtime/value.rs:24-104; brimstone/src/js/runtime/bytecode/cache.rs:19-31; boa/core/engine/src/vm/mod.rs:988-1010; boa/core/engine/src/vm/opcode/mod.rs:412; boa/core/engine/Cargo.toml:19-25; nova/ARCHITECTURE.md ('stack-based bytecode interpreter'); grep for cranelift/dynasm in Cargo.toml of boa, brimstone, nova, yavashark found nothing
- [high] V8 status on main (v15.8, commit 8f6ac1c, 2026-10-20). DEFINE_BOOL(turbolev, false) is described as 'use Turbolev (≈ Maglev + Turboshaft combined) as the 4th tier compiler instead of Turbofan'. turboshaft defaults to true. Tiering thresholds: feedback vector after 8 invocations, Maglev after 400 (1000 on Android), TurboFan after 3000, OSR 500, Maglev OSR 100. `--max-opt` 0/1/2/3 selects Ignition/Sparkplug/Maglev/TurboFan. Turbolev may still be field-trialed in Chrome; I did not verify that.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/v8-flag-definitions.h:1808 (turbolev), :1736 (turboshaft), :1121-1147 (thresholds), :917-929 (max_opt); v8 include/v8-version.h major 15 minor 8
- [high] Turboshaft replaced Sea of Nodes for the whole TurboFan JS backend and the whole Wasm pipeline. Stated reasons: SoN was too complex, too limited (most JS nodes sit on effect/control chains), and too slow. Results: compile time halved, load elimination up to 190x faster, about 3x fewer L1d misses. The JS frontend is being replaced by Maglev's CFG IR, which is the Turbolev path.
  EVIDENCE: https://v8.dev/blog/leaving-the-sea-of-nodes (2025-03-25)
- [high] Maglev is an SSA/CFG mid tier that consumes feedback (shape check + LoadField, 'Known Node Information'). It compiles about 10x slower than Sparkplug and 10x faster than TurboFan. Sparkplug compiles in one linear pass over bytecode with no IR, keeps the interpreter's frame layout so OSR is 'almost zero frame translation', and gave +45% JetStream / +41% Speedometer over Ignition alone.
  EVIDENCE: https://v8.dev/blog/maglev (2023-12-05); https://v8.dev/blog/sparkplug
- [high] JSC per-bytecode latency, measured by JSC on JetStream 2: LLInt 3.97 ns, Baseline 1.71 ns, DFG 0.349 ns, FTL 0.225 ns. DFG compile time is about 4x Baseline's, and FTL about 6x DFG's. An OSR exit costs about 2.5 µs (DFG) or 10 µs (FTL), so JSC only speculates when the profiled probability is above about 0.9994. Profiling inputs are case flags, value profiles, array profiles, IC 'negative cost profiling' and watchpoints. LLInt is written in offlineasm, and the 64-bit .asm sources total about 7k lines today.
  EVIDENCE: https://webkit.org/blog/10308/speculation-in-javascriptcore/; gh api WebKit/WebKit Source/JavaScriptCore/llint: LowLevelInterpreter.asm 3066 lines + LowLevelInterpreter64.asm 3920 lines; offlineasm backends arm64, arm64e, x86, riscv64, cloop
- [high] Current JSC thresholds are thresholdForJITAfterWarmUp=500, thresholdForOptimizeAfterWarmUp=1000 and thresholdForFTLOptimizeAfterWarmUp=64000 (the 2020 blog said 100000). JSC is building a new baseline replacement called 'LOL' (Light Optimization Layer) JIT, which has its own register allocator. It started 2025-12-19 and is off by default (useLOLJIT=false).
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/jsc-OptionsList.h:368,377,507; WebKit Source/JavaScriptCore/lol/LOLJIT.h; commit 'Add the first part of the new Light Optimization Layer JIT' 2025-12-19
- [high] JSC's FTL left LLVM for B3/Air because LLVM took more than 75% of FTL compile time and was 4.7x slower than B3 on hot JetStream functions. LLVM patchpoints also needed a predetermined size, and LLVM required parsing stackmap blobs. B3 has native Patchpoint and Check opcodes plus ValueRep for OSR exits. Measured results: Kraken +8-12%, Octane +5% on a MacBook Air.
  EVIDENCE: https://webkit.org/blog/5852/introducing-the-b3-jit-compiler/
- [high] SpiderMonkey tiers are the C++ interpreter, the Baseline Interpreter, Baseline JIT and Warp/Ion, with warm-up thresholds 10/100/1500. The Baseline Interpreter is generated by the same BaselineCodeGen as the Baseline JIT, shares JitScript, ICs and frame layout with it, and moved Speedometer from 31 to 52 (C++ interpreter → Baseline Interpreter), then to 69 with the Baseline JIT. Warp takes a snapshot on the main thread (WarpOracle) and builds MIR off-thread by transpiling CacheIR. It gained about 10-12% on Speedometer and 20% on Google Docs load. Bailouts resume in the Baseline Interpreter.
  EVIDENCE: https://hacks.mozilla.org/2019/08/the-baseline-interpreter-a-faster-js-interpreter-in-firefox-70/; https://hacks.mozilla.org/2020/11/warp-improved-js-performance-in-firefox-83/; https://firefox-source-docs.mozilla.org/js/how-we-optimize.html
- [medium] CacheIR is a linear IR of guards and ops with no branches. Constants such as shapes and offsets live in separate stub fields, so stub code is shared when the IR is identical. Baseline reads the stub fields, Ion bakes them in, and Warp transpiles the IR to MIR. SpiderMonkey's Portable Baseline Interpreter interprets CacheIR directly without generating code and keeps baseline-format frames on an auxiliary stack.
  EVIDENCE: https://jandemooij.nl/blog/cacheir/; https://kdab.searchfox.org/firefox-main/source/js/src/vm/PortableBaselineInterpret.h (via search summary)
- [high] Cranelift (0.136.4 stable, 0.137 rc) has user stack maps, but the CLIF producer must spill live GC refs to stack slots itself, and all non-tail calls are safepoints. It also has try_call/try_call_indirect exception tables, the Tail and PreserveAll call conventions, patchable calls (ExtFuncData.patchable), interrupt_poll, debug tags, sequence_point, an inliner, and the regalloc options backtracking or single_pass. It has no deopt or OSR-exit stackmap that gives value locations in registers, which is what B3's ValueRep provides.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/wasmtime/cranelift/codegen/src/ir/user_stack_maps.rs:1-30; cranelift/codegen/src/isa/call_conv.rs:14-67; cranelift/codegen/meta/src/shared/instructions.rs:307,342,376,3519; cranelift/codegen/src/ir/debug_tags.rs:1-30; cranelift/codegen/meta/src/shared/settings.rs:34-44; cranelift/codegen/src/inline.rs:143-147 (wasmtime commit fa20b4c, 2026-10-20)
- [medium] Measured Cranelift compile speed on baseline-JIT-shaped CLIF (per bytecode: 2 loads, NaN-box int32 tag test, sadd_overflow, store, slow-path call; about 19 CLIF insts per bytecode, 1000 bytecodes). opt=none + single_pass: 11 µs/bytecode (585 ns/inst, 146 B code per bytecode). opt=none + backtracking: 16 µs. opt=speed + backtracking: 21 µs (92 B per bytecode). Emitting raw AArch64 words for the same pattern takes 4.2 ns/bytecode, so Cranelift is about 2,600-5,000x slower than template emission (IR build time not included).
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/clif-speed/src/main.rs:12 (build), :76 (regalloc), :120 (template); output of ./target/release/clif-speed 1000
- [medium] Published Cranelift code quality: about 2% slower than V8 TurboFan and about 14% slower than LLVM (WAVM) on Wasm, with roughly 10x faster compiles than LLVM. These figures come from a 2020 paper as reported by Wasmtime and LWN.
  EVIDENCE: https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/README.md ; https://lwn.net/Articles/964735
- [high] On Apple Silicon (macOS 27, adhoc or linker-signed Rust binary), an RWX mmap without MAP_JIT fails with EACCES. With MAP_JIT it succeeds, and pthread_jit_write_protect_np(0) write, then (1) plus sys_icache_invalidate, then call works (it returned 42). A W^X toggle pair costs about 23 ns. Apple's porting doc (URL below) is the source for the hardened-runtime entitlement com.apple.security.cs.allow-jit; the page renders with JS, so I could not quote it.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/mapjit/main.rs:11-35 and its output; https://developer.apple.com/documentation/apple-silicon/porting-just-in-time-compilers-to-apple-silicon
- [high] dynasm-rs (dynasmrt 5.1.0) supports x64/x86 except AVX-512, aarch64 up to ARMv8.4 except SVE, and riscv32/64. Its last push was 2026-07-22. iced-x86 1.21.0 is x86 only.
  EVIDENCE: gh api repos/CensoredUsername/dynasm-rs/readme; cargo info dynasmrt / iced-x86
- [high] JSC value encoding uses NaN-boxing with raw pointers. Pointers are 0000:PPPP:PPPP:PPPP, so no decode is needed for cells. Doubles are stored with +2^49 (DoubleEncodeOffset). Int32 is FFFE:0000:IIII:IIII (NumberTag 0xfffe000000000000). isCell is !(v & NotCellMask).
  EVIDENCE: gh api WebKit/WebKit Source/JavaScriptCore/runtime/JSCJSValue.h lines 345-393, 724, 744-755
- [high] V8 pointer compression uses 32-bit tagged slots in a 4 GB-aligned region, with decompression as base + offset (2 instructions on x64), 31-bit Smis, and doubles boxed as HeapNumber. It cut V8 heap by up to 43% and renderer memory by up to 20%. Costs on Octane include about 1% from losing 32-bit Smis and about 3% from losing double field unboxing. Boxed doubles still cost V8 in 2025: mutable heap-number and int32 context slots gave about 2.5x on async-fs and +1.6% on JetStream2.
  EVIDENCE: https://v8.dev/blog/pointer-compression ; https://v8.dev/blog/mutable-heap-number (2025-02-25)
- [medium] Boa's NaN-boxing asserts that pointers are '4-bits aligned' and not 'over 51-bits', and it ships a jsvalue-enum fallback for platforms where that fails (issue #4275). V8's JS dispatch table carries special cases for LA57 (57-bit) user addresses. A NaN-boxing design must keep heap addresses within its payload bits, for example through mmap hints or reserved regions.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/boa/core/engine/Cargo.toml:19-25; gh api v8/v8 src/sandbox/js-dispatch-table.h comments lines 103-140
- [high] V8 object model: HiddenClass/Map points to a DescriptorArray; same property-add order yields a shared Map through a transition tree; in-object vs out-of-object property storage; dictionary mode on churn, where 'inline caches don't work'. Elements are stored separately in 21 elements kinds (a lattice of PACKED/HOLEY over SMI, DOUBLE and ELEMENTS) with one-way transitions.
  EVIDENCE: https://v8.dev/blog/fast-properties ; https://v8.dev/blog/elements-kinds
- [high] V8 string representations today are SeqOneByte, SeqTwoByte, Cons (rope, kMinLength=13), Thin (forwarding after internalization), Sliced (kMinLength=13), External one-byte/two-byte, and Internalized. The string_slices flag is read-only true.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/v8-string.h:900-1350,1100,1210; v8-flag-definitions.h (string_slices)
- [high] Calling-convention lesson from V8: removing the arguments adaptor frame (args pushed in reverse so parameters sit at fixed FP offsets, missing args padded with undefined, argc kept in the frame header) gave +11.2% in jitless mode, up to +40% with TurboFan on a microbenchmark, and +4.6% Richards / +6.1% EarleyBoyer.
  EVIDENCE: https://v8.dev/blog/adaptor-frame (2021-02-15)
- [high] V8 jitless works because builtins and bytecode handlers are generated at build time and embedded in the binary's .text ('embedded builtins'), with no runtime executable memory. The cost is about 40% on Speedometer 2 (half from the optimizing compiler, half from the regexp interpreter) and 80% on Web Tooling, but only 6% on a real YouTube app. In my measurements, V8 --jitless is 1.33x QuickJS on Octane and 2.80s vs 4.30s on the TypeScript workload.
  EVIDENCE: https://v8.dev/blog/embedded-builtins (2018-08-14); https://v8.dev/blog/jitless (2019-03-13); octane-run.log, ts-run.log
- [medium] Register vs stack VM: a register VM executes about 46-47% fewer VM instructions with about 25% larger bytecode, and was about 32% faster (1.48x on AMD64 switch dispatch). Ignition is a register machine with an implicit accumulator; its bytecode is 25-50% the size of baseline machine code. The CGO 2015 paper by Rohou et al. argues dispatch misprediction is small on TAGE/ITTAGE-class predictors when handlers are small, but Ertl disputed this on Haswell.
  EVIDENCE: https://usenix.org/legacy/events/vee05/full_papers/p153-yunhe.pdf ; https://mural.maynoothuniversity.ie/10185/1/KC-Virtual-2008.pdf ; https://v8.dev/blog/ignition-interpreter ; https://www.python.org/pipermail/python-dev/2015-August/141172.html
- [medium] Deegen (Xu & Kjolstad, arXiv 2411.11469) generates both an interpreter and a copy-and-patch baseline JIT from one semantic description. Its interpreter is 179% faster than PUC Lua and 31% faster than LuaJIT's hand-written-asm interpreter. Its baseline JIT has 'negligible startup delay' and runs only 33% slower than LuaJIT's optimizing JIT. This is evidence that generated interpreters plus templated baselines are a high-leverage design.
  EVIDENCE: https://arxiv.org/abs/2411.11469
- [medium] Source size of each V8 tier (bytes of .cc/.h/.tq on main), as a rough guide to effort: baseline/Sparkplug 0.4 MB, interpreter 1.1 MB, ic 0.7 MB, deoptimizer 0.4 MB, maglev 4.4 MB, compiler (TurboFan + Turboshaft) 13.7 MB of which turboshaft 3.5 MB, codegen/assemblers 8.1 MB. The baseline tier is cheap to build; the mid tier is about 10x larger, and the top tier about 3x larger again.
  EVIDENCE: gh api repos/v8/v8/git/trees/main?recursive=1 -> /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/v8-tree.json (aggregated sizes)
- [medium] Hermes V1 (formerly 'Static Hermes') became the default React Native engine in 0.84 (2026-02-11). It is a mobile and AOT-bytecode design, not a V8/JSC-class JIT reference. QuickJS (bellard, version 2026-06-04) now builds with CONFIG_TEMPORAL_INTL, which is relevant to the spec/Intl topic.
  EVIDENCE: https://reactnative.dev/blog/2026/02/11/react-native-0.84 (via search); /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/qjs-build.log (-DCONFIG_VERSION="2026-06-04" -DCONFIG_TEMPORAL_INTL)

## Risks
- Benchmark noise: the machine was shared with other agents (load average about 11 on 18 cores). I mitigated this with best-of-3 runs and per-process isolation, but expect ±10-15% on individual cells. Octane measures steady state and favors peak tiers. Node 24.22 ships V8 13.6, not 15.8 main; Bun 1.4.3 ships JSC from WebKit commit 0c06faad.
- The dispatch microbenchmark is dispatch-bound with tiny handlers, so it overstates the gain from tail-call dispatch. Real JS handlers are larger; CPython reports 3-5%, while Wasmi's 1.6x is near the top of the range.
- Relying on stable sibling-call optimization is fragile. Any handler that takes the address of a local, needs stack-passed arguments, or calls an alloca-ing function silently loses the tail call and grows the stack. This needs CI enforcement through disassembly checks and deep-loop tests. Debug builds must use the match loop.
- Nightly-only `become` and `rust-preserve-none` can change or regress, and their tracking issues list open soundness and ABI work (indirect arguments, signature checks in PR #156007). Mixing a nightly-only interpreter crate into a workspace pinned to stable 1.99.1 (tsrs) is a toolchain problem.
- NaN-boxing assumes heap addresses fit in 47-48 bits. LA57 x86 and 52-bit-VA arm64 kernels, ASLR, and sanitizers can break that unless address-space reservation is controlled. The choice also rules out V8-style pointer compression memory savings (up to 43% heap) unless the whole design changes later.
- Cranelift has no deopt stackmaps (B3 ValueRep-style), and its user stack maps require explicit spills. Using it for a speculative tier forces deopt exits to be calls with all live values, which costs code size and register pressure. Its compile cost (11-21 µs/bytecode) also rules it out for the baseline and probably the mid tier.
- JIT security: type-confusion bugs in speculative tiers are the main exploit class in V8, JSC and SpiderMonkey. This needs differential fuzzing between tiers (Fuzzilli-style) from S3 onward, W^X discipline, and probably CFI on code entry points (V8 write-protects JSDispatchTable).
- Apple hardened runtime or notarized distribution needs the com.apple.security.cs.allow-jit entitlement. I only verified adhoc-signed binaries locally. iOS (no third-party JIT outside special cases) would need the Rust interpreter or a build-time-embedded generated interpreter.
- dynasm-rs has a single maintainer and supports aarch64 only up to ARMv8.4, so a production engine will probably need its own encoders eventually (V8 and JSC have them).
- Effort scale: the V8 mid tier (Maglev) is about 4.4 MB of source and TurboFan + Turboshaft about 13.7 MB. The S4/S5 targets are multi-person-year efforts even with a simpler design.

## Open questions
- GC coupling, which needs reconciling with the GC research: a moving or generational GC plus precise stack maps for JIT frames, or JSC-style conservative native-stack scanning? Conservative scanning fits NaN-boxing with raw pointers and greatly simplifies rooting in Rust runtime code, but it pins objects.
- Should interpreter frames live on a separate VM stack (natural for a Rust interpreter; SpiderMonkey PBL does this) or on the native stack (needs a generated interpreter)? This decides how cheap interpreter↔JIT OSR and deopt can be.
- Is a JIT-free mode (iOS, locked-down hosts) a product requirement? If yes, the Rust interpreter must stay first-class, or a generated interpreter must be embedded at build time, as V8 embedded builtins are.
- Toolchain policy: pin nightly for `become` + preserve_none (about 1.3-1.5x on dispatch-bound code), or stay on stable with CI-verified sibling calls until stabilization (targeted for 2027)?
- Is Turbolev enabled in Chrome through field trials despite the false flag default (flag-definitions.h:1808)? Not verified.
- I did not measure absolute Sparkplug or Maglev compile cost per bytecode (V8 does not trace Sparkplug compile times). The Cranelift comparison against them rests on the blog's 10x/10x ratios.
- Does the tsrs AST carry enough scope and binding resolution for a register-allocating bytecode generator, or must ep9 run its own scope analysis? That belongs to the parser research topic, but it gates S1.

## Artifacts
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/src/main.rs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/cgoto.c
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/hyperfine-dispatch.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/dispatch/x86.s
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/mapjit/main.rs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/clif-speed/src/main.rs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/scripts/run_matrix.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/scripts/summarize.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/scripts/run_ts.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/octane-run.log
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/octane-table.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/octane-results.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/ts-run.log
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/v8-flag-definitions.h
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/jsc-OptionsList.h
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/v8-string.h
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/bench
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/bench2/octane-typescript.cjs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/quickjs/qjs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/target-boa/release/boa
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/target-bs/release/bs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/exec-pipeline/target-nova/release/nova_cli
