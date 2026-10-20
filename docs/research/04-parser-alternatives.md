# parser-alternatives

## Summary
I measured six front ends with my own harness. It runs every test262 test at commit d53ebc8 (2026-10-13), except staging, through each parser, and it times parsing on real files. oxc_parser 0.155 plus oxc_semantic with syntax checks on is the only library that catches 100% of test262 parse-phase early errors (4653/4653), and it accepts 99.99% of valid tests. boa_parser 0.22 catches 99.79%, Brimstone's own parser 99.87%, swc 66.7%, the tsrs parser alone 26.8%, and tsrs parser plus binder 42.0%. Adding the binder also brings 868 false errors, because TypeScript 7 treats all code as strict. Most early errors in tsrs live in the 108k-line tsrs_checker (61 check_grammar_* functions), not in the parser. Raw speed: oxc parse-only runs at 320-680 MB/s and parse+semantic at 170-350 MB/s. tsrs runs at 115-350 MB/s on .js files, because in JS mode it eagerly parses JSDoc; that costs 2.3x on lodash and 3.8x on antd. In TS mode tsrs reaches 380-600 MB/s, but parsing JS as TS changes meaning: `a < b > (c)` becomes a generic call. swc runs at 150-370 MB/s, Brimstone parse+analysis at 130-265 MB/s, and boa at 12-28 MB/s. None of the Rust library parsers (oxc, swc, boa, tsrs) has a V8/SpiderMonkey/JSC-style preparser or a way to re-parse one function with its saved context. Each engine works around this. Nova (built on oxc) keeps the whole oxc arena, plus scope data, alive in its GC heap and compiles each function to bytecode on its first call. Brimstone and Boa compile every function up front and then drop the AST. Nova also hit oxc gaps that are still open: no eval-context parse options (oxc issue #11793, open since 2025-06), no API to parse Function() parameters and body separately, a strict script has to be parsed twice, and it has stayed pinned at oxc =0.124 since April 2026 while oxc reached 0.155. No public sign exists that oxc and tsrs plan to share an AST; they only share lint policy and some copied number-conversion code. For a production engine aimed at speed, my ranking is: (1) an engine-owned, hand-written front end in the V8 style; (2) oxc as the front end you start with now, behind a lowering boundary you own; (3) a fork of only the tsrs scanner, not the parser; (4) boa_parser; (5) swc. Keep tsrs for type checking and tooling, not for the engine's parse path.

## Recommendation
Ranked for a production, speed-focused engine whose author strongly prefers tsrs.

1. Build an engine-owned, hand-written front end (option b, the V8/SpiderMonkey/JSC model).
   - One-pass recursive-descent parser with scope analysis built in, producing a bytecode-oriented AST that is dropped after codegen.
   - A syntax-only "preparse" mode that does not build an AST but still raises every early error and records captured variables.
   - First-call re-parse of a function from a saved context (strict, function/method/derived-constructor/class-field flags, private names, scope info).
   - Built-in eval/Function() context entry points.
   - WTF-8 or Latin-1/UTF-16 input.
   - Brimstone shows the size: about 20k lines including the regexp parser, and 99.87% of test262 early errors.
   - For the tsrs preference: port only tsrs_scanner (6.8k lines, Apache-2.0, fast). Update its Unicode tables to 17, and add sloppy mode and Annex B lexing (HTML comments, legacy octal).
   - Grammar code can be borrowed from tsrs's parser and checker, but the engine owns it.
   - Parse TS syntax and skip types natively in this parser, as Nova does with oxc, so .ts runs without a separate transpile pass.

2. Use oxc (parser + semantic + regular_expression + ecmascript) as the front end you start with now.
   - Best measured conformance (100% of early errors, 99.99% of valid tests) and the fastest parser. MIT, WTF-8 strings, full TS syntax, and Nova proves it works.
   - Put it behind a lowering boundary you own: lower oxc AST plus Scoping into your own IR/bytecode at once and free the arena. Do not keep NonNull pointers into it as Nova does.
   - Pin it exactly.
   - Run oxc and your own parser side by side on the test262 harness, and use oxc as the reference while building option 1.
   - Known gaps to plan for: no preparse, eval context (#11793), Function() parameters/body, a strict script needs a double parse, and high API churn.

3. Fork tsrs (option e) only as a scanner fork, not a parser fork. Taking tsrs_parser means also taking tsrs_ast and tsrs_core (arena, P<T>, global registry), about 100k lines. On top of that you would have to:
   - add sloppy mode and Annex B;
   - recover the roughly 3,400 early errors the parser misses (73%);
   - remove eager JSDoc parsing for JS;
   - write engine scope analysis (the TS binder is built for types);
   - keep a permanent fork, because upstream PRs are closed and the code mirrors TS-Go line by line.

4. boa_parser: about the same conformance as oxc and accepts UTF-16 input, but 10-25x slower, Box AST, no TS.

5. swc: catches only 66.7% of early errors, no regex validation, 21 major versions in 12 months.

6. tsrs as a dependency in the engine's parse path: not suitable, because it rejects valid sloppy code, catches 27-42% of early errors, has 2-4x JSDoc overhead on .js, and its exact Unicode pin breaks other crates in the same build.

Keep tsrs for what it does best: the runtime's `check`/LSP tooling, and the TS-to-JS transform for enums, namespaces and decorators if wanted. Run it as a separate binary or process so its exact dependency pins do not reach the engine's build.

## Findings
- [high] test262 parse-phase results (all of test/language, annexB, built-ins and intl402 except _FIXTURE files; positives = valid tests that must parse, negatives = parse-phase tests that must fail). oxc parser+semantic: 47357/47364 positives (99.99%) and 4653/4653 negatives (100%). oxc parser alone: 70.11% of negatives, so about 30% of early errors are caught only by oxc_semantic. boa 0.22 with annex-b: 99.58% / 99.79%. Brimstone parser+analyze with annex_b: 98.81% / 99.87%. swc 46.1.1: 99.57% / 66.69%. tsrs parser: 99.91% / 26.78%. tsrs parser+binder: 98.17% / 41.97%. None crashed.
  EVIDENCE: Harness scratchpad/work/frontend-alternatives/common/t262.rs. Per-parser failure lists are in .../frontend-alternatives/results_clean/*.fails.txt (oxc, boa, brimstone) and .../results/{swc,tsrs-parser,tsrs-parser+binder}.fails.txt. test262 commit d53ebc8b1931d9f1541e43d59650776e4b766248.
- [high] oxc's 7 remaining false errors are all new Annex B tests where `f() = 1` must be a runtime error, not an early error. swc misses all regex-literal early errors and the RegExp property-escape errors (334 tests), plus about 495 class-related ones, and it rejects Annex B function declarations in if-statements. boa's misses are mostly in staged proposals (import defer, using / await using).
  EVIDENCE: grep FALSE/MISSED in results_clean/oxc-parser+semantic.fails.txt, results/swc.fails.txt, results_clean/boa.fails.txt
- [high] oxc only gives full early errors if you also run SemanticBuilder::with_check_syntax_error(true) and set ParseOptions.parse_regular_expression=true. With the regex option off, my oxc+semantic run caught 92.3% instead of 100%.
  EVIDENCE: oxc_parser-0.155.0/src/lib.rs ParserReturn doc: 'This list is not comprehensive. Oxc offloads more-expensive checks to semantic analysis'. Rerun after enabling parse_regular_expression moved negatives from 4295 to 4653.
- [high] Most ECMAScript early errors in tsrs are in the type checker, not the parser: break/continue targets, for-in/of heads, let declarations, 'use strict' with non-simple parameters, object-literal rules, regex literal validation, yield/await, and more.
  EVIDENCE: third_party/tsrs/crates/tsrs_checker/src/grammarchecks.rs has 3615 lines and 61 `fn check_grammar_*`, e.g. :131 check_grammar_regular_expression_literal and :2440 check_grammar_break_or_continue_statement. The tsrs_checker crate is about 108.7k lines.
- [high] The tsrs binder runs strict-mode checks with no condition, because TypeScript 7 is always strict. As a result `with`, `eval`/`arguments` used as binding names, and duplicate sloppy-mode declarations are rejected in sloppy scripts. Running the binder caused 868 false errors on valid test262 tests: 288 TS1101, 230 TS1100, 119 TS2300, 114 TS1212.
  EVIDENCE: tsrs_binder/src/binder.rs:1024 `Kind::WithStatement => self.check_strict_mode_with_statement(node)` with no strict-mode test, and :2250. The binder is called from the harness at .../frontend-alternatives/harness/src/main.rs (tsrs_parse2).
- [high] The tsrs parser on its own rejects valid sloppy-mode and Annex B JS: HTML-like comments, legacy octal literals and escapes, `await` as an identifier in scripts, `let` as an identifier in a for head, `a?.b.#x` (TS18030), and `for (let [a = b in c] of ...)`.
  EVIDENCE: results/tsrs-parser.fails.txt FALSE_ERROR lines, e.g. annexB/language/comments/*html* (TS1109), language/literals/numeric/legacy-octal-integer.js (TS1121), language/statements/let/syntax/escaped-let.js (TS1260)
- [high] tsrs pins unicode-id-start to exactly 1.1.2 (Unicode 15.1) to match TypeScript's tables, so it rejects the test262 Unicode 16 and 17 identifier tests. Cargo applies that pin to the whole build: when tsrs was in the same build, oxc was silently downgraded to Unicode 15.1 and failed 16 tests it passes on its own. Adding swc 46 to the same build failed outright, because swc needs unicode-id-start ^1.2.
  EVIDENCE: third_party/tsrs/Cargo.toml:40-41. Combined harness Cargo.lock resolved unicode-id-start 1.1.2; the oxc-only harness resolved 1.5.0. Compare results/ (contaminated) with results_clean/oxc-parser+semantic.fails.txt. cargo fetch error: 'failed to select a version for unicode-id-start ... required by swc_ecma_ast v29.0.2'.
- [high] Median parse throughput on Apple aarch64 (MB/s, release build, thin LTO, mimalloc; Brimstone uses its own jemalloc):
- react: oxc 491, oxc+sem 278, tsrs-JS 352, swc 265, Brimstone 241, boa 23
- lodash: oxc 679, oxc+sem 349, tsrs-JS 212-248, swc 368, Brimstone 264, boa 25
- three: oxc 321, oxc+sem 177, tsrs-JS 225, swc 152, Brimstone 132, boa 14
- echarts: oxc 316, oxc+sem 168, tsrs-JS 221, swc 157, Brimstone 133, boa 12
- antd: oxc 500, oxc+sem 299, tsrs-JS 116, swc 227, Brimstone 205, boa 28
- typescript.js (10.9 MB): oxc 444, oxc+sem 251, tsrs-JS 289-331, swc 216, Brimstone 187, boa 19
- checker.ts: oxc 476, oxc+sem 249, tsrs-TS 366, swc 228
  EVIDENCE: .../frontend-alternatives/bench_js.txt, bench_ts.txt, bench_bs.txt. Fixtures are the oxc benchmark set from jsdelivr, in .../frontend-alternatives/fixtures.
- [high] In JS mode, tsrs eagerly parses every JSDoc comment and synthesizes type nodes from it (reparse_tags). This makes .js parsing 2.3-3.8x slower. Parsing JS as TS avoids the cost but changes meaning: in TS `a < b > (c)` is a generic call, in JS it is two comparisons.
  EVIDENCE: tsrs_parser/src/jsdoc.rs:78-127: TS files defer JSDoc, JS files parse it eagerly and call reparse_tags. bench_tsrs_jsdoc.txt: lodash 2.26 ms (JS) vs 0.91 ms (TS mode); antd 54.8 vs 14.9 ms. The ambiguity was shown with oxc (harness/examples/ambig.rs): JS gives BinaryExpression, TS gives CallExpression.
- [high] None of oxc, swc, boa or tsrs offers a preparser or a re-parse-one-function-with-context API. Re-parsing a cut-out function with oxc gives false errors, because the private-name and super context is gone.
  EVIDENCE: harness/examples/reparse.rs: the full class parses clean; re-parsing `() => this.#x + super.y` alone gives "Private identifier '#x' is not allowed outside class bodies" and a 'super' error. grep finds no lazy/preparse in oxc_parser/src. tsrs's only laziness, parse_member_list_lazily (tsrs_parser/src/parser_1.rs:1019), still parses fully, then rewinds the arena and records context; it covers .d.ts member lists only and saves memory, not parse time.
- [medium] V8, SpiderMonkey and JSC all use a lightweight syntax-only parse for inner functions, which still detects all early errors and does not build an AST, and they fully parse a function on its first call. V8's preparser must also track variable references to decide which variables live in heap contexts. SpiderMonkey's SyntaxParseHandler header calls it 'several times faster than a full parse'. JSC's Parser is templated on ASTBuilder vs SyntaxChecker and caches function info in SourceProviderCache.
  EVIDENCE: https://v8.dev/blog/preparser ; https://firefox-source-docs.mozilla.org/js/ and searchfox js/src/frontend/SyntaxParseHandler.h ; gh api repos/WebKit/WebKit/contents/Source/JavaScriptCore/parser lists SyntaxChecker.h, ASTBuilder.h, SourceProviderCache.h
- [high] Nova consumes oxc as follows. It parses into an oxc Allocator and runs SemanticBuilder with check_syntax_error. It then uses transmute to stretch the Program, AstNodes and Scoping lifetimes to 'static and stores the Allocator inside the GC-managed SourceCodeHeapData. Function objects keep raw NonNull pointers into the oxc AST, and bytecode is compiled lazily on the first call. The whole AST arena therefore lives as long as any function from that source.
  EVIDENCE: nova/nova_vm/src/ecmascript/scripts_and_modules/source_code.rs:151 (Allocator::new), :262-284 (transmutes plus heap.create), :311-323 (SourceCodeHeapData owns allocator). ecmascript_function.rs:90-96 (NonNull<oxc_ast::ast::Function<'static>>). function_definitions.rs:236-241 (compile_function_body on first call). Nova HEAD 877ef16.
- [high] Problems Nova hit with oxc:
- Strict scripts are parsed twice: once sloppy, then again as a module to force strict mode.
- oxc has no eval-context options (inFunction, inMethod, inDerivedConstructor, inClassFieldInitializer, privateNames, strict). Issue #11793, filed by the Nova maintainer, is still open.
- There is no API to parse Function() parameters and body separately, so Nova parses one combined script and checks that it is a single statement.
- Non-UTF-8 JS source strings go through to_string_lossy, so lone surrogates in eval source are probably replaced.
- Nova's regex runtime uses the Rust `regex` crate; oxc_regular_expression only checks syntax.
- Nova needed WTF-8 strings from oxc (oxc#16886).
  EVIDENCE: source_code.rs:155-205 and :97. function_constructor.rs:224-226 ('oxc doesn't have a public API to do that'). gh issue 11793 oxc-project/oxc (OPEN, 2025-06-18). regexp/data.rs:6. oxc issue 16886 comment by Boshen: 'Nova requires it'.
- [high] oxc's API changes constantly: 60 breaking 0.x releases of oxc_parser in the last 12 months, and 110 [BREAKING] entries in its CHANGELOG between 0.124 and 0.155. Nova had about 25 bump PRs, then switched to exact (=) pins in 2026-04 and has stayed on 0.124. Since then oxc renamed ParserReturn.errors to diagnostics and panicked to fatal_error, added node_id: Cell<NodeId> to every node, and made AstNodes opt-in (into_scoping_and_nodes returned 0 nodes in my test). Andromeda must use the same oxc version as its pinned Nova revision. swc_ecma_parser went from major 25 to 46 in 12 months, and Deno's deno_ast pins swc_ecma_parser =27.0.7.
  EVIDENCE: crates.io versions API. oxc/crates/oxc_parser/CHANGELOG.md. gh pr list trynova/nova (bump PRs #782-#975), nova commit 'use strict versions (#980)' 2026-04-12, nova/Cargo.toml:35-43. oxc_parser-0.155.0/src/lib.rs:165-211. andromeda/Cargo.toml:44-57. deno_ast 0.53.3 dependencies.
- [medium] With oxc, keeping the AST alive (as Nova does) costs about 4.6x the source size in arena memory, plus Scoping. typescript.js (10.9 MB) uses a 49.9 MB arena; peak RSS is 63 MB for parse only and 87 MB with semantic. tsrs regions use a similar amount (52.7 MB).
  EVIDENCE: harness/examples/retain.rs measured with /usr/bin/time -l; the 'mem' column in bench_js.txt
- [high] Brimstone wrote its own front end of about 20k lines: lexer, recursive-descent parser, a separate analyze pass that builds a scope tree and assigns each binding a VMLocation (Argument, LocalRegister, Scope slot, ModuleScope, EvalVar, WithVar), and its own regexp parser. It uses a bumpalo arena AST over WTF-8 source and has parse_script_for_eval with eval context. It has NO lazy parsing: all functions are compiled from a queue, then the AST is dropped. No public statement of why it wrote its own parser was found; the README says it implements 'almost all components ... from scratch with minimal dependencies'.
  EVIDENCE: brimstone/src/js/parser/*.rs (wc: 20359 lines). scope_tree.rs:1389-1406. parser.rs:5086-5135 (parse_script, parse_script_for_eval). runtime/bytecode/generator.rs:91, :199-206. runtime/context.rs:282-306. README.md lines 1-15. HEAD 64d6698.
- [high] Boa's parser: boa_parser takes a ReadChar source, either UTF-8 bytes or UTF-16 (Source::from_utf16, the only Rust library parser here with UTF-16 input). It builds an owned Box-based boa_ast with identifiers interned as Sym. parse_script also runs AST scope analysis (analyze_scope). Annex B is a cargo feature. The engine compiles nested functions eagerly. License: Unlicense OR MIT. It is 10-25x slower than oxc in my benchmark.
  EVIDENCE: boa_parser-0.22.0/src/parser/mod.rs:145-176, source/mod.rs:39,61, Cargo.toml [features] annex-b. boa/core/engine/src/bytecompiler/mod.rs:2454-2479 (FunctionCompiler run inline). bench_js.txt
- [medium] How runtimes handle TS:
- Node strips types with amaro 1.2.1, a wrapper around @swc/wasm-typescript in strip-only mode, and feeds the JS to V8.
- Deno transpiles with deno_ast, which is swc-based.
- Bun transpiles with its own parser (bun_js_parser, now Rust, under src/js_parser) and gives JS text to JSC.
- JSC and V8 do not parse TS.
Only Nova/Andromeda compile TS straight from the AST: Nova's `typescript` feature has 148 cfg sites, and the bytecode compiler ignores TSAs/TSSatisfies/TSNonNull etc. and handles enums.
  EVIDENCE: npm info amaro (1.2.1), amaro README. deno Cargo.toml:95 deno_ast =0.53.3 with swc deps. gh api oven-sh/bun src/js_parser (Cargo.toml name bun_js_parser, p.rs 453 KB). nova bytecode_compiler.rs:2740-2755; grep count of feature="typescript" = 148.
- [medium] No public evidence that oxc plans to adopt the tsrs AST or the reverse. The oxc repo at HEAD does not mention tsrs, and a GitHub issue search across the oxc-project org finds tsrs only in tsrs's own repo. tsrs reuses oxc's Clippy policy and copied oxc's number conversion code at the owner's request. tsrs describes itself as an 'unpublished, mechanical Go port', has publish=false, and does not accept PRs.
  EVIDENCE: grep -rni tsrs on the oxc clone (aa7642a) gives 0 hits. gh search issues --owner oxc-project tsrs: 12 results, all in oxc-project/tsrs. tsrs Cargo.toml:115-140. notes/perf-oxc-numbers.md:1-19. README 'Pull requests remain closed'.
- [high] The tsrs AST does not fit an engine well. Nodes are a uniform 24-byte header (packed parent pointer, kind, data tag, AtomicU32 id) with kind-specific data; code reaches it through P<Node> handles and checks node.kind(), not typed enums. Memory comes from a per-thread leak arena or Regions. Source text goes into a global registry whose slots are never reused.
  EVIDENCE: tsrs_ast/src/ast.rs:540-563. tsrs_core/src/arena.rs:1-23. tsrs_parser/src/parser_1.rs:213-300 (parse_source_file_owned docs: 'never freed', 'slots are never reused').
- [high] Licenses: oxc MIT, swc Apache-2.0, boa Unlicense OR MIT, Brimstone MIT, Nova and Andromeda MPL-2.0, tsrs Apache-2.0 as a TypeScript derivative (NOTICE attribution needed).
  EVIDENCE: oxc/Cargo.toml:11, crates.io license fields, brimstone/LICENSE, nova/LICENSE.md, tsrs README License section

## Risks
- The test262 runner is approximate. It checks parse-phase only (negatives = `phase: parse`). The strict variant is made by prepending "use strict". `raw` tests run as sloppy. Feature flags are not filtered. Resolution and runtime negatives are ignored. Percentages could shift by a few tenths compared with the official runners.
- Results in .../results/ for oxc came from a build where tsrs's unicode-id-start pin downgraded oxc. Use .../results_clean/ for oxc, boa and Brimstone. swc and tsrs numbers come from builds that are valid for them.
- In tsrs, `diagnostics()` being non-empty is counted as a parse error. Under ScriptKind::JS some valid-JS diagnostics may be TS-policy errors that a fork could turn off. The 26.8% figure reflects tsrs as it ships, not a patched fork.
- Benchmarks are single-process medians on one macOS aarch64 machine. Brimstone uses jemalloc and the others mimalloc. boa's time includes scope analysis and interning. oxc parse-only does no scope analysis, while Brimstone's numbers include its analyzer, so compare Brimstone with oxc+sem.
- The claims about V8, SpiderMonkey and JSC lazy parsing come from blog and docs summaries and file listings, not from reading their source in depth.
- Bun's parser has just been ported to Rust (bun_js_parser). I only checked the repo layout, not how its runtime is wired.
- Whether oxc and tsrs plan to share an AST could be decided in private channels (Discord, the backlog repo) that I cannot see. I found no public evidence either way.
- Nova's lossy handling of lone surrogates in eval source comes from reading the code (to_string_lossy); I did not test it.

## Open questions
- Would the tsrs maintainers (same org) accept engine-oriented options upstream, such as parsing JSDoc only on demand in JS mode, a sloppy-mode/Annex B switch, or a configurable Unicode version, even though PRs are closed? Only the owner can answer this.
- Would oxc add the eval-context parse options (#11793), a Function() parameters/body API, and a preparse or lazy-function mode if an engine asked? That would greatly improve option 2.
- How much engine-specific scope information (registers vs heap contexts, captured-variable flags) can be read from oxc_semantic Scoping alone, and how much needs a second analysis pass? Nova's variable_escapes_scope (bytecode_compiler.rs:587) is one data point.
- What preparse speed do we target? A preparser that only checks syntax could plausibly reach 2x full-parse speed, but nobody has measured that in Rust. A prototype on top of the tsrs scanner would answer it.

## Artifacts
- scratchpad/work/frontend-alternatives/common/t262.rs
- scratchpad/work/frontend-alternatives/harness/src/main.rs
- scratchpad/work/frontend-alternatives/harness/examples/reparse.rs
- scratchpad/work/frontend-alternatives/harness/examples/ambig.rs
- scratchpad/work/frontend-alternatives/harness/examples/retain.rs
- scratchpad/work/frontend-alternatives/harness_oxc/src/main.rs
- scratchpad/work/frontend-alternatives/harness_swc/src/main.rs
- scratchpad/work/frontend-alternatives/harness_bs/src/main.rs
- scratchpad/work/frontend-alternatives/results_clean/oxc-parser+semantic.fails.txt
- scratchpad/work/frontend-alternatives/results_clean/boa.fails.txt
- scratchpad/work/frontend-alternatives/results_clean/brimstone.fails.txt
- scratchpad/work/frontend-alternatives/results/tsrs-parser.fails.txt
- scratchpad/work/frontend-alternatives/results/tsrs-parser+binder.fails.txt
- scratchpad/work/frontend-alternatives/results/swc.fails.txt
- scratchpad/work/frontend-alternatives/bench_js.txt
- scratchpad/work/frontend-alternatives/bench_ts.txt
- scratchpad/work/frontend-alternatives/bench_bs.txt
- scratchpad/work/frontend-alternatives/bench_tsrs_jsdoc.txt
- scratchpad/repos/nova
- scratchpad/repos/brimstone
- scratchpad/repos/oxc
- scratchpad/repos/test262
