# rust-engine-landscape

## Summary
Pass rates below are from test262.fyi, generated 2026-10-20T02:44Z on test262 d53ebc8 (53,616 tests). QuickJS 98.48% (its Intl and Temporal support is AI-written and landed 2026-10-10), LibJS 97.72%, Boa 95.59%, Kiesel 94.78%, Porffor 91.17%, QuickJS-ng 83.51%, Nova 76.92%, Hermes 41.80%. Brimstone, Yavashark, engine262 and GraalJS are not on test262.fyi. My local run puts Brimstone at about 98% of the standard language tests and 95.5% of built-ins, but only about 84% of the full suite, because it skips Intl, Annex B and staging. Performance spread is huge. On my local v8-v7 run, scores are: node --jitless 4644, Brimstone 4137, QuickJS 3168, Boa 516, Yavashark 220 and Nova 193 (Nova failed the RegExp test). zoo.js.org arm64 data puts full V8 and JSC at about 25x QuickJS and GraalJS at about 4.3x. Every high-compliance engine except Nova (oxc) wrote its own parser. Nova (oxc) and Yavashark (swc) needed workarounds around their external parsers. The biggest regrets all come from performance-critical choices made late and retrofitted after compliance work: Boa's GC rooted by reference counts, its stack VM and its enum JsValue; Kiesel's Boehm GC and old VM; Brimstone's switch from AST walker to bytecode at month 13; LibJS's 2023 JIT, removed in 2024 because its interpreter was 5x slower than JSC's. The reusable pieces with real adoption are temporal_rs (used by V8, Boa, Kiesel, Nova and Brimstone), ICU4X, regress, ryu-js, fast-float2 and xsum. Benchmarks should be JetStream 3.0 (released 2026-03-31, runs in a shell via cli.js with a JSC-style host API), the v8-v7/Octane subset for daily tracking, and the Web Tooling Benchmark (already inside JetStream as the *-wtb workloads). Speedometer needs a DOM and cannot run on a bare engine. Most important 2026 change: AI-assisted work has cut the cost of the compliance and porting work that has an oracle. Examples: QuickJS added 69k lines of Intl/Temporal; Ladybird ported 25k lines to Rust in 2 weeks; Porffor went from 65% to 92% in 9 days. JIT and GC architecture is still where the slow, expert-judgment cost sits.

## Recommendation
Concrete lessons, ordered by how expensive they are to change later:

1) Decide the performance architecture on day 1, not after compliance. Every engine that deferred it paid with a rewrite:
   - Boa: AST to VM in 2022; stack to register VM and NaN-boxing in 2025; GC still pending in 2026.
   - Brimstone: AST walker to bytecode at month 13; whole-runtime handle refactor at month 6.
   - Kiesel: VM rewrite in year 3.
   - Starlight: died during a VM+GC rewrite.

   Fix these up front:
   - (a) Value representation: NaN-boxing over a heap you reserve yourself (a cage or offsets, as LibJS does). Avoid raw malloc pointers: Boa's Android pointer-tag panic is still open.
   - (b) Precise, movable-capable GC with safepoint rooting enforced by the type system. Use a branded &mut token or handle scopes (Nova's GcScope, Boa's 2026 plan, V8/Hermes HandleScope). Never use rooting by reference counts: Boa measured 10-18% mutator overhead. Avoid conservative Boehm-style GC: Kiesel's profiles were dominated by GC and used 9GB of memory.
   - (c) Register bytecode plus scope analysis putting locals in registers (Boa +25%).
   - (d) Shapes plus polymorphic ICs whose data is a shared IR (CacheIR-style) that later JIT tiers consume. Kiesel got 20-30% from ICs in a few hundred lines; Brimstone's 2026 ICs took it past QuickJS.
   - (e) Interpreter frames laid out so a Sparkplug-style baseline JIT can reuse them.

2) Target: the interpreter tier should first reach about V8-jitless speed, which is about 1.5-1.7x QuickJS. Brimstone shows one person can get to about 0.9x V8-jitless. Build the JIT only after that point, the same order LibJS ended up with. The order that works:

```
parser+early errors -> test262 runner (parallel, per-test timeout) -> heap/value/GC rooting -> register bytecode
  -> builtins by test262 directory -> shapes+ICs -> generated/asm interpreter -> baseline JIT -> optimizing JIT
```

3) Parser: every high-compliance engine except Nova (oxc) owns its parser. External parsers needed workarounds: Nova's strict-mode double parse and AST arena kept alive with transmutes; Yavashark's 5k-line early-error validator. If tsrs is used, budget for an early-error/strict-mode/regexp validation layer and for AST lifetime and lazy-compilation memory. The tsrs-specific gaps were not verified here; they belong to the parser track.

4) Reuse instead of rebuild:
   - temporal_rs (12.4% of test262; V8 uses it; plan 7-12k lines of glue).
   - ICU4X 2.x (Intl). Note that Boa and Kiesel are still about 80% on intl402 after years, while QuickJS reached 99.7% with an AI-written, self-contained implementation.
   - regress, or your own regexp later for speed; never the `regex` crate.
   - ryu-js, fast-float2, xsum, num-bigint.

5) Leave out the features that cost Hermes compliance (direct eval, `with`, mapped sloppy `arguments`, Annex B function hoisting, Symbol.species) only from the fast path, never from the engine. Design the scope analysis so these deopt only the affected functions.

6) Benchmarks:
   - Daily: the v8-v7 suite (Boa CI) for cross-engine comparison with Boa, Kiesel, QuickJS and V8-jitless.
   - Per change: Octane 2 plus JetStream 3.0 JS-tagged workloads (includes ARES-6, Octane, Web Tooling, SunSpider) through cli.js. This needs a JSC-shell host API (runString/loadString/load/readFile/print/performance.now/setTimeout); copy Brimstone's tests/perf harness.
   - Reference shells via jsvu, with --jitless / useJIT=0 to get interpreter baselines.
   - No Speedometer without a DOM.

7) Effort estimates, in person-years (PY), for an expert team. Pre-AI calibration comes from Brimstone, Kiesel, LibJS and Boa; the 2026 AI factor comes from QuickJS, Ladybird and Porffor.

| Milestone | Estimate | Calibration |
|---|---|---|
| ES+Annex B ≥97% in a well-architected interpreter | 3-5 PY pre-AI; maybe 1.5-3 PY with AI help, since test262 is an oracle | 1-person hobby projects took about 3 years |
| + Intl/Temporal to ≥97% | +1-2 PY, or +0.3-1 PY with temporal_rs/ICU4X plus AI | |
| Fast interpreter (≥V8-jitless) | +2-4 PY, mostly object model, ICs, a generated or assembly interpreter, GC tuning; little speedup from AI | |
| Baseline JIT, x64+arm64, production-hardened | 1.5-3 PY | Sparkplug was about 1 core engineer for about 1 year on mature infrastructure; the LibJS 2023 JIT was removed after 4 months; Hermes' arm64 JIT has been ongoing since 2024 |
| Optimizing JIT competitive with V8/JSC (about 15x jitless) | 15-40+ PY plus a permanent security/fuzzing staff | Maglev alone was about 19 months with about 6 core people on top of TurboFan/deopt infrastructure; about 45% of V8 CVEs were JIT-related |

   The 64k-line LibJS Rust JIT landed in October 2026 with AI help; its maturity is unproven, so treat it as an upper bound on speed, not a norm.

## Findings
- [high] Current test262.fyi full-suite pass rates (53,616 tests, test262 d53ebc8, generated 2026-10-20): SM-exp 98.85, JSC 98.80, QuickJS 98.48, SM 98.46, V8-exp 98.34, LibJS 97.72, V8 97.69, Boa 95.59, Kiesel 94.78, Porffor 91.17, QuickJS-ng 83.51, XS 81.39, Nova 76.92, Hermes 41.80, NJS 35.29. GraalJS, engine262, Brimstone, Yavashark and Starlight are not in the v3 data.
  EVIDENCE: https://data.test262.fyi/index.json and meta.json, saved under scratchpad/work/engine-lessons/t262_index.json and t262_meta.json; percentages computed locally
- [high] Per-area breakdown: Boa language 98.1 / built-ins 96.0 / intl 79.0; Kiesel 95.1 / 98.5 / 80.2 (Annex B 58.9); Nova 95.5 / 70.6 / 0.8; QuickJS-ng 98.2 / 79.4 / 0.7; Porffor 92.0 / 92.1 / 98.7. Nova's low built-ins score comes from Temporal (13%), RegExp (50%) and Iterator (35%).
  EVIDENCE: data.test262.fyi index.json, built-ins.json, built-ins/RegExp.json, built-ins/Temporal.json
- [high] Test262 composition drives strategy. Temporal is 6,634 tests (built-ins/Temporal 4,605 plus intl402/Temporal 2,029), or 12.4% of the suite. intl402 has 3,382 tests in total. Class tests are 8,426 (expressions/class 4,059 plus statements/class 4,367), about 16%. dynamic-import has 1,005. annexB has 1,086 and staging 1,483.
  EVIDENCE: data.test262.fyi intl402.json, language/expressions.json, language/statements.json, built-ins.json
- [high] QuickJS added 'experimental AI coded Temporal and Intl support' on 2026-10-10 (commit 97f64ef, +69,094 lines with no external dependencies, including about 24k lines of locale data). Nine days later test262.fyi shows QuickJS at 99.7% intl402 and 100% built-ins/Temporal.
  EVIDENCE: gh api repos/bellard/quickjs/commits/97f64ef96f19a47a23665f4d19c4a689e9a1e42c (stats.additions=69094); test262.fyi intl402.json
- [high] Ladybird ported LibJS's lexer, parser, AST and bytecode generator to Rust (about 25k lines) in about 2 weeks with Claude Code and Codex. The bytecode output is byte-for-byte identical, with 0 test262 regressions. Kling said the same work would have taken 'multiple months' by hand. By 2026-10-20 LibJS has about 216k lines of Rust in Libraries/LibJS/Rust/src, the 'Flap' interpreter DSL compiler (49k lines plus a 4,951-line interpreter.flap, since 2026-07-24), and a 64k-line Rust optimizing JIT crate whose first commits are dated 2026-10-10.
  EVIDENCE: https://ladybird.org/posts/adopting-rust/ ; sparse clone scratchpad/repos/ladybird-libjs-sparse @b47d69f (wc -l per directory); gh api commits?path=Libraries/LibJS/JIT/Rust (earliest 2026-10-10) and path=Libraries/LibJS/Flap (2026-07-24)
- [high] The LibJS JIT design (2026) has no baseline tier: 'The interpreter's caches are the baseline'. It has two tiers: a bytecode interpreter with per-site caches and feedback, and an SSA optimizing JIT. The JIT compiles snapshots on compile threads, supports on-stack replacement (OSR) and deopt exits back to the interpreter, uses one forward-walk register allocator, and checks every op against an effects model.
  EVIDENCE: Libraries/LibJS/JIT/ARCHITECTURE.md (saved at scratchpad/work/engine-lessons/libjs/JIT_ARCHITECTURE.md), sections 1-3
- [high] LibJS removed its first JIT on 2024-02-19, about 4 months after it started in 2023-10. The commit says: 'security & complexity cost ... far too high' and 'JavaScriptCore ... ~5x faster than us on benchmarks when running without a JIT. We need to catch up to them before considering performance techniques with a heavy security cost.'
  EVIDENCE: Ladybird commit 1d29f9081fc243407e4902713d3c7fc0ee69650c; first LibJS/JIT commits 2023-10-16
- [high] LibJS timeline: first commit 2020-03-07. test262 pass rate 43.5% (2021-06), 74.8% (2023-01), 94.8% (2024-01), 96.8% (2025-07), 98.2% (2026-07). The AST interpreter was gone by 2023-10. LibJS uses a non-moving mark-sweep GC (LibGC) with conservative stack and register scanning, plus NaN-boxing.
  EVIDENCE: raw.githubusercontent.com/LadybirdBrowser/libjs-data/master/test262/results.json (20,105 runs); SerenityOS commit 'LibJS: Start building a JavaScript engine' 2020-03-07; Libraries/LibGC/Heap.cpp gather_conservative_roots; LibGC/NanBoxedValue.h
- [high] Boa timeline (first commit 2018-08-08; 302 author emails, 1,434 of the commits by dependabot): 18.0% (v0.10, 2020-10), 49.3% (v0.14, 2022-03), 74.4% (v0.16, 2022-09), 85.1% (v0.18, 2024-03), 94.1% (v0.21, 2025-10), 95.6% (v0.22, 2026-08). About 8 years with a volunteer team to reach about 95%.
  EVIDENCE: raw.githubusercontent.com/boa-dev/data/main/test262/refs/tags/<tag>/latest.json; gh api repos/boa-dev/boa/releases; git log of the full clone scratchpad/repos/boa-dev_boa
- [high] Boa retrofitted its performance architecture late. AST walker to bytecode VM in 2022 (v0.15), when a single format!() call in the hot path caused a regression. Scope analysis putting locals in registers in 2024-12/2025-03 (+25% overall, +70% on one benchmark). Stack VM to register VM (PR #3798) and NaN-boxing by default, both in v0.21 (2025-10). Its GC is a fork of rust-gc that 'we have long been pushing to its limits'.
  EVIDENCE: boa-dev.github.io blog posts 2022-06-10-boa-release-15.md, 2025-03-05-local-variables.md, 2025-10-22-boa-release-21 (saved in scratchpad/work/engine-lessons/boablog)
- [high] Boa's own GC measurement (issue #5567, 2026-10-10): reference-count rooting costs 10-18% of mutator time, because Gc/JsValue cannot be Copy and every move runs drop glue plus a thread-local read. Collection costs 35-50% on splay. The recommended fix is GC only at interpreter safepoints, plus a branded &mut GcToken so the borrow checker enforces rooting (similar to Nova's GcScope). Boa rejected gc-arena because of re-entrancy, and the oscars prototype drifted back to reference counting to avoid use-after-free.
  EVIDENCE: gh issue view 5567 -R boa-dev/boa; boa-dev/oscars PR #68; core/gc/src/internals/gc_header.rs (ref_count plus non_root_count)
- [high] Boa's NaN-boxing assumes pointers fit in 48 bits and panics on Android aarch64 ('Pointer is not 4-bits aligned or over 51-bits', issue #4275, opened 2025-06-07 and still open). The enum JsValue remains as a fallback feature. Lesson: allocate the JS heap in your own reserved region (a cage, or offsets as LibJS's heap_region_offset does) instead of boxing raw malloc pointers.
  EVIDENCE: gh issue view 4275 -R boa-dev/boa; core/engine/Cargo.toml feature jsvalue-enum; core/engine/src/value/inner/nan_boxed.rs; LibJS interpreter.flap box_object uses heap_region_offset
- [high] Nova (first commit 2022-08-24; 854 commits, Aapo Alasuutari 419 of them): stack-based bytecode, oxc parser plus oxc_semantic for early errors, data-oriented struct-of-arrays heap with 32-bit handles, a compacting GC, and borrow-checker-enforced safepoint rooting (GcScope/NoGcScope with bind/unbind/scope). RegExp uses the `regex` crate, which has no lookaround or backreferences. 1.0 shipped 2026-03-15. Activity since then: 8, 4, 1, 0, 0, 0 and 2 commits per month (April to October 2026).
  EVIDENCE: scratchpad/repos/trynova_nova: README.md, ARCHITECTURE.md, GARBAGE_COLLECTOR.md, Cargo.toml (oxc_* =0.124.0, regex =1.13.1); git log; trynova.dev/blog/nova-1.0
- [medium] Nova's documented regrets. The author asks whether they 'should've just worked with only rooted values from the get go'. The code is 'filled to the brim' with .unbind()/.bind(gc.nogc()). Newcomers unroot handles just to silence the borrow checker. The work was funded by a 6-month NLnet grant and reached about 77%. The 1.0 post says 'it is not a fast engine'.
  EVIDENCE: trynova.dev/blog/contrarian-thinking-part-2, /blog/worked-for-the-internet, /blog/nova-1.0
- [high] Using an external parser in an engine (Nova plus oxc) needs workarounds. Strict scripts and eval are parsed twice (once sloppy, then again as a module to get strict mode). AST lifetimes are transmuted to 'static, and the bump allocator is kept alive in a self-referential heap object for the life of the code. Small eval strings are padded with 10 spaces to force heap allocation. SemanticBuilder.with_check_syntax_error(true) adds a second pass for early errors. Yavashark (swc) needed its own 5,057-line early-error validator, started 2025-09-15.
  EVIDENCE: nova_vm/src/ecmascript/scripts_and_modules/source_code.rs lines ~120-260; Sharktheone_yavashark/crates/yavashark_swc_validator/src/lib.rs (header comment) and its git log
- [high] Parser source per engine. Own parser: Boa, Brimstone, Kiesel, LibJS (now Rust), QuickJS (one-pass straight to bytecode), Hermes, engine262, GraalJS, and Porffor (acorn until the 2026-07-15 rewrite, now its own 4.7k-line JS+TS parser). External parser: Nova (oxc), Yavashark (swc plus validator), Starlight (swc).
  EVIDENCE: Cargo.toml/build.zig.zon of each clone; CanadaHonk_porffor/compiler/parser/index.js header; Porffor README at commit 785b15a~ (acorn); zoo.js.org engines.json parser field
- [high] Brimstone (single author, first commit 2022-11-18, 1,319 commits, 123k lines of Rust). Own parser, Ignition-style register bytecode VM (added 2023-12-03, replacing the AST walker after 13 months), Cheney semispace copying GC with V8-style HandleScope handles (written in 'very unsafe Rust'), pointer-favoring NaN-boxing, own RegExp engine, ICU4X and temporal_rs. A whole-runtime refactor to GC-safe handles happened in 2023-05. Polymorphic inline caches and fast paths were added in 2026-07..09.
  EVIDENCE: scratchpad/repos/Hans-Halverson_brimstone: README.md, src/js/runtime/gc/garbage_collector.rs:20, gc/handle.rs, value.rs:24-63, git log
- [medium] Brimstone test262, measured locally (HEAD 64d6698 against test262 d53ebc8, Brimstone's own harness and ignore lists). Built-ins 22,496/23,565 = 95.5%: SharedArrayBuffer, Atomics and explicit resource management are missing; Iterator is 435/654. Language about 22,208/22,568 = 98.4%, not counting language/expressions/dynamic-import (1,005 tests), where a test hung for over 11 minutes and the harness has no per-test timeout. intl402 and Annex B are skipped as non-standard, and staging is 16/1,478. Roughly 84% of the full 53,616-test suite.
  EVIDENCE: scratchpad/work/engine-lessons/bs_test262_builtins.txt, bs_test262_expr.txt, bs_test262_lang.txt, bs_test262_dirs.txt; tests/test262/ignored_tests.jsonc
- [medium] Local v8-v7 suite (Boa's CI version: Richards, DeltaBlue, Crypto, RayTrace, EarleyBoyer, RegExp, Splay, NavierStokes), one run each on this Apple Silicon machine, 2026-10-20. node --jitless (Node 24.22) 4644; Brimstone 64d6698 4137 (0.89x V8-jitless, 1.31x QuickJS); QuickJS e2dc3eb 3168; Boa 9871775 516 (0.16x QuickJS); Yavashark 94d77df (tree-walker default) 220; Nova 877ef16 193 on 7/8 suites (RegExp threw 'look-around ... is not supported').
  EVIDENCE: scratchpad/work/engine-lessons/benchv8_results.txt; run_benchv8.sh; benchv8/combined_all.js. Binaries for boa/bs/nova/qjs were built by a sibling agent (work/exec-pipeline) from the same HEADs; a concurrent sibling benchmark adds noise
- [high] Boa's CI (same v8-v7 suite, GitHub runner, 2026-10-20): Boa 226, Kiesel 410, Duktape 374, sm-jitless 768, QuickJS 1219, Porffor (AOT) 2556, v8-jitless 2560. LibJS was 840 vs QuickJS 858 on 2026-01-16 and is no longer tracked after that. QuickJS jumped about 37% in mid-2026, matching its 2026-06-04 release note: '42% faster ... bench-v8'. The note credits a custom small-block malloc (+11%) and micro-optimizations (+30%).
  EVIDENCE: raw.githubusercontent.com/boa-dev/data/main/bench/results/score.json (saved as boa_score.json); bellard_quickjs/Changelog 2026-06-04; https://bellard.org/quickjs/
- [medium] zoo.js.org arm64 Octane-style geomean ratios vs V8-jitless (data Dec 2025 to Mar 2026): V8 14.98, JSC 14.97, SpiderMonkey 7.61, GraalJS 2.56, Hermes-v1 (formerly Static Hermes) 1.08, Escargot 0.75, JSC-jitless 0.73, Hermes 0.68, QuickJS 0.59, PrimJS 0.56, QuickJS-ng 0.51, LibJS 0.37, ChakraCore 0.33, SM-jitless 0.31, Duktape 0.31, Brimstone (Dec 2025, before ICs) 0.13, XS 0.12, Boa 0.105, Kiesel 0.088, Nova 0.046, Yavashark 0.022 (6/15 tests), engine262 0.001 (4/15).
  EVIDENCE: https://zoo.js.org/engines.json (saved as zoo_engines.json); geomean of per-test ratios over shared completed tests, computed locally
- [high] Kiesel (Zig; Linus Groh wrote 2,832 of 2,885 commits; started 2023-04-28). Reuses bdwgc (conservative Boehm GC), QuickJS libregexp, ICU4X via icu4zig and temporal_rs, and has had a bytecode VM since week 3 (2023-05-15). Devlog #10 (2024-08-19): 'virtually all time was spent in garbage collection'; v8 combined.js went from 50 minutes to 5 and peak memory from 9GB to under 2GB; Boa score 33 to 56 vs QuickJS 900. In 2026 the VM was rewritten as a register VM with an IR (devlog #13), and polymorphic ICs gave +20-30% in a few hundred lines (devlog #15, 2026-10-08).
  EVIDENCE: codeberg kiesel README and build.zig.zon; git log of scratchpad/repos/kiesel-js_kiesel; https://linus.dev/posts/kiesel-devlog-10/ , -13/, -15/
- [medium] Porffor (AOT JS to C; essentially one author, CanadaHonk / Oliver Medhurst). Its own test262 history plateaued at about 60% from 2025-06 to 2026-03. It was fully rewritten on 2026-07-15 (+33k/-31.6k lines, own parser, C only), then went from 64.71% (2026-10-01) to 92.05% (2026-10-19); 58 commits landed on 2026-10-19 alone. An AI policy requiring disclosure was added on 2026-07-24. Whether AI drove the jump is not stated.
  EVIDENCE: CanadaHonk_porffor/test262/history.json; git show --shortstat 785b15a7; AI_POLICY.md; git log date counts
- [high] Hermes deliberately breaks the spec for speed and AOT compilation: no local (direct) eval, no `with`, no runtime ES module loader, no Symbol.species, unmapped sloppy `arguments`, and localeCompare ignores locale. Its GC is Hades (generational, with a concurrent old generation), with handle-based rooting (GCScope/PinnedHermesValue). An arm64 baseline-style JIT has existed since 2024-08 (about 400KB of C++). It scores 41.8% on test262.fyi.
  EVIDENCE: facebook/hermes doc/Features.md, doc/SpecIncompat.md, doc/Hades.md, doc/GCSafeCoding.md, doc/VM.md; lib/VM/JIT/arm64 file sizes; first JitEmitter.cpp commit 2024-08-27
- [high] QuickJS architecture: one-pass parser emits stack bytecode directly (no AST). Reference counting plus trial-deletion cycle collection (gc_decref/gc_scan). JSValue is a 16-byte tag+union on 64-bit; NaN-boxing only on 32-bit. First public release 2019-07-09. Upstream was dormant 2021-2023 (1, 4, and then 57 commits per year), which led to the QuickJS-ng fork (2023-11). ByteDance's PrimJS replaced reference counting with a tracing GC and added a template interpreter, claiming +28% Octane over QuickJS.
  EVIDENCE: bellard_quickjs/quickjs.h:55-65,145,216-232; quickjs.c:6677-6840; Changelog tail; git log commits/yr; quickjs-ng README; lynx-family/primjs README
- [high] Starlight is dead (last commit 2021-10-31, after 598 commits in 9 months) but not formally archived on GitHub. Its README says the dev branch 'will not build'. In its final months it rewrote the VM, started the Comet Immix GC and added a Cranelift 'fullcodegen' plus a tracing-JIT directory, while relying on swc and nightly Rust.
  EVIDENCE: scratchpad/repos/Starlight-JS_starlight README.md, crates/*/Cargo.toml (cranelift 0.75, swc_ecmascript 0.52, comet immix), git log; gh api repos/Starlight-JS/starlight (archived=false, pushed 2021-10-31)
- [medium] Yavashark (one author 'Shark', 4,206 of 4,230 commits since 2024-03). swc parser; the default execution mode is the tree-walking interpreter, with a bytecode VM behind a feature. GC is reference counting with cycle detection. Its README claims about 81% of test262 (self-reported, not on test262.fyi). It is the slowest engine measured here.
  EVIDENCE: Sharktheone_yavashark README.md, src/run.rs:110-126, crates/yavashark_garbage/README.md, Cargo.toml (swc_ecma_parser 45); benchv8_results.txt
- [high] engine262 is a spec-shaped TypeScript AST interpreter (about 69k lines), built for proposal prototyping. It skips intl402, Annex B, SharedArrayBuffer/Atomics and tail calls, and lists 83 known failures. It is about 1000x slower than V8-jitless (zoo.js.org).
  EVIDENCE: engine262_engine262 README.md, test/test262/{features,failed,skip}; zoo_engines.json
- [medium] GraalJS (Truffle AST interpreter plus Graal partial-evaluation JIT on the JVM) lists only 30 FAIL and 434 SKIP overrides in its test262 expectation file, which suggests near-complete compliance (not independently run). On zoo.js.org it is about 4.3x QuickJS.
  EVIDENCE: raw.githubusercontent.com/oracle/graaljs/master/graal-js/test/test262.json (saved as graal_test262.json); zoo_engines.json
- [high] temporal_rs (boa-dev/temporal) is used by Boa, Kiesel and V8 for core Temporal functionality, and also by Nova and Brimstone. It has 9.8M crates.io downloads (7.0M recent). Integration glue is still large: Boa about 11.7k lines, Kiesel about 11.1k lines of Zig, Brimstone about 7.3k lines. Nova has 3.3k lines and passes only 13% of built-ins/Temporal.
  EVIDENCE: gh api repos/boa-dev/temporal README; crates.io API; wc -l of builtins/temporal directories in each clone; test262.fyi built-ins/Temporal.json
- [high] Reusable crates with adoption. regress 0.12 (backtracking ES regexp with UTF-16/UCS-2 modes; Boa scores 99.8% on built-ins/RegExp). ICU4X 2.3 (icu_* crates, used by Boa and Brimstone; boa_icu_provider ships compiled data). ryu-js (shortest Number-to-string). fast-float2 and lexical (string to number). xsum (Math.sumPrecise). num-bigint (Boa, Nova, LibJS-Rust). boa_string, small_btree and tag_ptr (small Boa utilities). ecmascript_atomics and soavec (Nova). oxc_regular_expression (parser only). Avoid the `regex` crate for JS RegExp.
  EVIDENCE: crates.io API output (versions and downloads); Boa Cargo.toml; Nova Cargo.toml; LibJS Rust/Cargo.toml; Nova RegExp failure in benchv8_results.txt
- [high] Build order seen in git history. Brimstone: parser, test262 runner and NaN-boxed managed heap (month 1), GC-safe handles refactor (month 6), ICU4X (month 9), bytecode VM (month 13), ICs (year 4). Kiesel: Boehm GC (day 1), bytecode VM (week 3), register VM rewrite (year 3). Boa: AST walker (2018), VM (2022), Intl (2022), Temporal (2024-25), register VM and NaN-boxing (2025), GC rewrite (still pending in 2026). LibJS: AST walker (2020), bytecode (default by 2023), JIT added (2023-10) and removed (2024-02), Rust frontend (2026-02), Flap assembly interpreter (2026-07), optimizing JIT (2026-10).
  EVIDENCE: git logs of the clones (grep for bytecode/gc/icu); Ladybird commits cited above; Boa blog posts
- [high] JIT effort data points. Sparkplug was upstreamed 2021-02-12 by Leszek Swirski (32 of the early commits), shipped in Chrome 91, and is 'a switch statement inside a for loop' that relies on Ignition-compatible frames and shared builtins (+5-10% Speedometer). Maglev's initial commit was 2022-02-24 and it shipped in Chrome M117 (blog 2023-12-05), with 6 named core authors and 24-41 distinct committers per year. It is CFG SSA rather than sea-of-nodes, reuses TurboFan's deoptimizer, and compiles about 10x slower than Sparkplug and 10x faster than TurboFan. SpiderMonkey's Warp (Firefox 83, 2020) builds MIR from CacheIR, the same inline-cache IR used by the Baseline Interpreter and Baseline JIT.
  EVIDENCE: gh api repositories/24420506/commits?path=src/baseline (last page) and path=src/maglev; https://v8.dev/blog/sparkplug ; https://v8.dev/blog/maglev ; https://hacks.mozilla.org/2020/11/warp-improved-js-performance-in-firefox-83/
- [medium] JIT security cost: in 2021 Microsoft reported that about 45% of V8 CVEs (post-2019) were JIT-related and that more than half of in-the-wild Chrome exploits relied on JIT bugs ('Super Duper Secure Mode').
  EVIDENCE: https://therecord.media/microsoft-announces-new-super-duper-secure-mode-for-edge/ ; windowscentral coverage
- [high] JetStream 3.0 was released 2026-03-31 as a joint Apple, Google and Mozilla effort. The JetStream3.0 branch exists, and main became 'JetStream Next' on 2026-08-31 (PR #300). In a shell, cli.js needs a JSC-style host API: load(path), readFile(path), print/printErr, runString('') returning a fresh global with loadString (or d8 Realm / SpiderMonkey newGlobal), performance.now, setTimeout, globalThis.arguments, and optionally gc(). Run `npm run decompress` first. Wasm-tagged workloads can be excluded, since tags separate 'js' from 'Wasm'. ARES-6, Octane, SunSpider and the Web Tooling Benchmark (*-wtb) are included as tagged workloads. Brimstone's tests/perf harness (an --expose-test-shell-compat flag, pinned commits) is a working template.
  EVIDENCE: WebKit/JetStream cli.js, utils/shell-config.js, JetStreamDriver.js:762-810 and tag counts (saved in scratchpad/work/engine-lessons/jetstream); gh api branches; PR #300; https://browserbench.org/announcements/jetstream3 ; Hans-Halverson_brimstone/tests/perf/{install.sh,README.md,src/suites/jetstream.rs}
- [high] Octane was retired by V8 on 2017-04-12 because tuning for it caused 'overly-specific heuristics', and chromium/octane is archived. It is still the common cross-engine yardstick for interpreters (Boa CI, zoo.js.org, Kiesel). Speedometer needs a browser DOM; for a shell-only engine the closest proxies are JetStream's web-ssr, jsdom-d3-startup and *-startup workloads.
  EVIDENCE: https://v8.dev/blog/retiring-octane ; gh api repos/chromium/octane (archived=true); JetStream tags
- [high] Running test262 fast matters. test262.fyi wall-clock times: QuickJS 145s, NJS 82s, Boa 298s, Kiesel 406s, SpiderMonkey 602s, LibJS 1,835s, Porffor 5,217s. Brimstone's harness runs most of the suite in seconds over 12 threads, but has no per-test timeout, so a single hung dynamic-import test blocked the whole run for 11+ minutes.
  EVIDENCE: t262_meta.json times.timeTaken; local ps -M of the harness (11:04 CPU on one thread, about 8.7s on the others)

## Risks
- My local v8-v7 numbers are single runs, on the same machine as a sibling agent's concurrent benchmark matrix; they carry ±10-20% noise. The ratios do agree with Boa's CI and with zoo.js.org.
- zoo.js.org data is from Dec 2025 to Mar 2026. Brimstone (ICs added 2026-07+) and QuickJS (+42% in 2026-06) have changed a lot since, so cross-source ratios mix dates.
- Brimstone's test262 numbers come from its own harness and ignore lists, run on test262 d53ebc8 rather than its pinned 227d905. The 1,005 dynamic-import tests did not finish because one test hung, so its share of the full suite (~84%) is an estimate.
- The test262.fyi Hermes score (41.8%) may be lowered by harness or flag choices on top of Hermes' deliberate deviations. Not investigated.
- That AI drove the jumps is verified for QuickJS (commit message) and Ladybird (blog). For Porffor it is not stated; only the velocity is verified. The maturity of the LibJS Rust JIT is unknown.
- The effort estimates in person-years are extrapolations from git history and public posts, not measured staffing. Contributor counts by git email overcount people (Boa includes dependabot; Porffor's two top authors are likely one person).
- The GraalJS expectation file shows overrides only. Reading 30 FAIL as near-complete compliance assumes every unlisted test passes, which I did not run.
- The libjs.dev domain no longer hosts LibJS content (the fetched page served unrelated third-party content), so I did not use it. LibJS history comes from the LadybirdBrowser/libjs-data repo instead.

## Open questions
- Does tsrs's parser report all ECMAScript early errors and handle sloppy/strict/module/eval goal symbols, Annex B grammar and regexp literal validation, given that the TypeScript compiler defers many grammar errors to the checker? This needs the parser track's verification.
- How much of LibJS's October 2026 Rust optimizing JIT is on by default, and what JetStream/Octane speedups does it give over the Flap interpreter? No public numbers were found yet.
- Will Boa's planned safepoint + branded GcToken GC (issue #5567) ship, and what speedup does it measure? It is the closest Rust precedent for the GC rooting design.
- Brimstone's current JetStream 3 results via its tests/perf harness were not run here; doing so would calibrate what a solo, interpreter-only Rust engine reaches on modern workloads.
- Exact staffing for Maglev, Sparkplug and the Hermes JIT was not found; the person-year figures rely on commit-author counts and blog bylines.

## Artifacts
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/run_benchv8.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/benchv8_results.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/benchv8/combined_all.js
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/benchv8/combined_all_printshim.js
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/bs_test262_builtins.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/bs_test262_expr.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/bs_test262_lang.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/bs_test262_dirs.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/t262_index.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/t262_meta.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/zoo_engines.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/boa_score.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/boa_t262
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/libjs_results.json
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/libjs/JIT_ARCHITECTURE.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/jetstream
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/hermes
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/boablog
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/engine-lessons/target-yava/release/yavashark
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/ladybird-libjs-sparse
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/kiesel-js_kiesel
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/boa-dev_boa
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/trynova_nova
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/Hans-Halverson_brimstone
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/Sharktheone_yavashark
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/Starlight-JS_starlight
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/CanadaHonk_porffor
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/engine262_engine262
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/quickjs-ng_quickjs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/bellard_quickjs
