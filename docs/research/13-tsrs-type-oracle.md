# tsrs-type-oracle

## Summary
Yes, with conditions. I built ep9_typecheck, a Rust binary that links tsrs by path (21 crates, 363k lines; 38 s clean build; 13.6 MB stripped). It type-checks a tsconfig project with one checker or with N checkers, then exports a side table of "typed IR hints" keyed by UTF-8 span. A second, oxc-only binary dumps oxc_parser 0.155 node spans, and a Python script joins the two by (start, end) plus a kind-compatibility table.

Join coverage:
- 99.94% of 556,724 oxc nodes on TypeScript's own src/compiler (78 files, 10 MB).
- 99.64% of 1.07M nodes on all of TypeScript src/ (602 files, 18 MB).
- 99.46% on a typed TS port of the AWFY Richards, DeltaBlue and NBody benchmarks that I wrote. It runs and self-verifies under node 24.
- 0 kind mismatches and 0 panics in all runs. Every leftover miss is a node that is not an expression on the tsrs side (import specifiers, quoted keys, `constructor`, object spread).
- One real hazard: tsrs strips a UTF-8 BOM, transcodes UTF-16 and lossily repairs bad UTF-8, which shifts offsets. Fixed in the prototype by stripping the BOM on the oxc side.

Cost:
- 10 MB of TS: check 0.46 s single-threaded; hint export 1.2 s; 223 MiB peak.
- 18 MB with 16 checkers: check 0.22 s; export 0.58 s; 0.87 s wall total.
- The single-checker and 16-checker side tables are byte-identical.

Usefulness:
- Typed class code gives near-total precision: 0.2% any, 100% of member accesses resolved, field slot found for 64% of them, 97% of calls with a resolved target.
- Idiomatic large TS is interface-based: 47% of receivers are structural interfaces, 32% of member accesses are enum constants, and slot hints apply to only 0.2%.
- checkJs works, but un-annotated JS is 39% any and JSDoc-typed Preact is 30% any. For Preact, 65% of member accesses resolve to hand-written .d.ts, not the implementation.
- The hints are unsound. 13 small holes all pass the checker, and the hint is wrong at runtime in every one: structural class types, array out-of-bounds, double casts, any from JSON.parse, method bivariance, covariant arrays, `!`, int vs double, a lying .d.ts return type, .d.ts field order opposite to the real order, delete through any, prototype patching, and property narrowing kept across a mutating call.

API: everything needed is pub, apart from one free function in a private module. The internals still drift: the oracle builds against the tsrs of 8 days earlier with a 2-line change, and against 15 days earlier with 6 compile errors. Linking tsrs and oxc in one binary pins oxc to Unicode 15.1, and tsrs brings a fixed 32 GiB address reservation, arenas that live until process exit, and about 220 globals.

Recommended consumption: a separate offline ep9-typecheck binary that writes a hashed, span-keyed side table. The AOT compiler treats every hint as a speculation that needs a guard, never as a fact.

## Recommendation
Use tsrs's checker as an OFFLINE oracle in a separate binary, never as trusted types and never linked into the engine.

Pipeline:
```
 tsconfig ──> ep9-typecheck (tsrs, N checkers) ──> side table per file
                                                     { decoded-text hash, records sorted by (start,end,kind):
                                                       category, class-id = decl file:offset, slot candidate,
                                                       call target decl, dts/lib flag, enum constant }
 source  ──> ep9 (oxc_parser + oxc_semantic) ──> join on (start,end)+kind ──> AOT IR with *speculative* types
                                                                         └─> guards: shape/class check, number/int check,
                                                                             null check; deopt or fallback on failure
```

Why a separate binary:
1. The unicode-id-start =1.1.2 pin would downgrade oxc.
2. The fixed 32 GiB address reservation, arenas that live until process exit, about 220 globals and abort paths stay out of the runtime.
3. Fast-moving internal APIs (6 compile errors over 2 weeks) sit behind one stable side-table schema that ep9 owns.
4. Apache-2.0 obligations (tsrs is a TypeScript derivative) stay in a build tool.
5. It already scales: 18 MB checked and exported in 0.87 s with 16 checkers, with output identical to a single checker.

Side-table rules:
- Store a hash of the decoded text, because of BOM, UTF-16 and lossy-UTF-8 decoding.
- Prefer the ParenthesizedExpression span for JSDoc casts.
- Never emit type strings.
- Use a binary format, about 20 B per node; the JSON is about 195 B per node.
- Drop or demote hints in files with type errors, and in nodes marked dts/lib (claims about foreign code).

Codegen policy:
- Treat each hint like an inline-cache seed. Emit a fast path for the hinted class shape or number kind behind a cheap guard, then fall back or deopt.
- Use hints for: slot candidates on class receivers (64% of member accesses in class-heavy code), monomorphic call targets plus CHA over the exported class hierarchy, enum constant folding (32% of member accesses in TypeScript's compiler), and numeric expressions (int vs double still needs ep9's own range analysis).
- Interfaces (47% of receivers in idiomatic TS) give no layout. Use them only to pre-size inline caches.

Optional later step: a Static-Hermes-like strict mode where ep9 itself enforces soundness (nominal classes, checked casts at any/.d.ts boundaries, bounds checks), so guards can be removed. It needs ep9's own checks, not tsrs changes.

For JS dependencies: checkJs with maxNodeModuleJsDepth ≥ 2 gives usable but weak types (30-39% any); plan on JIT-style feedback for them.

## Findings
- [high] A Program plus checker can be driven from an outside crate with path deps and no tsrs changes. Recipe: tsoptions::get_parsed_command_line_of_config_file -> new_cached_fs_compiler_host(cwd, Arc::new(bundled::embed::wrap_fs(osvfs::fs())), &bundled::embed::lib_path(), None, None, None) -> ProgramOptions::new(P::new(config), host) with single_threaded = Tristate::True|False -> new_program (returns a leaked &'static Program) -> get_diagnostics_of_any_program(ctx, program, None, false, bind cb, semantic cb), which forces the full check -> program.get_type_checker_for_file(ctx, sf) derefs to &mut Checker. The crate setup copies the earlier harnesses: an empty [workspace] table, tsrs's Cargo.lock copied in, and cargo +1.99.1. Lib .d.ts files are embedded through tsrs_vfs::bundled::embed.
  EVIDENCE: work/tsrs-type-oracle/oracle/src/main.rs (main); pattern taken from third_party/tsrs/crates/tsrs_testrunner/src/types_dump.rs:45-80 and type_symbol_baseline.rs:235-330; clean build 38.4 s wall / 132.5 s CPU (out/build1.log); 21 tsrs crates, 363,408 Rust lines in the closure; 13,592,272-byte stripped binary (lto off)
- [high] The query API is public: get_type_at_location (checker_15.rs:3179, flow-narrowed), get_symbol_at_location_exported (checker_15.rs:2401), get_apparent_type (checker_11.rs:553), get_non_nullable_type (checker_09.rs:2494), get_resolved_signature_exported (exports.rs:385) + sig.declaration + get_return_type_of_signature (checker_10.rs:1100), get_call_signatures / get_construct_signatures (services.rs:351/356), get_properties_of_type (checker_09.rs:2729), get_base_types (checker_09.rs:4060), is_array_type (checker_11.rs:3669), get_type_arguments (checker_11.rs:948), get_constraint_of_type_parameter (checker_08.rs:3315), get_constant_value (services.rs:1188), type_to_string_exported (printer.rs:59). Classification uses only TypeFlags (types.rs:880; NumberLike/StringLike/BooleanLike/BigIntLike/Union/Any/TypeParameter/Object) and ObjectFlags (types.rs:1048; Class, Interface, Reference -> t.target(), ObjectLiteral). An error type is detected with t == c.get_error_type(). The polymorphic `this` is a TypeParameter with is_this_type() (types.rs:2881) and is classified through its constraint. The only compile failure was checker::is_tuple_type: it is pub but sits in a pub(crate)-reexported module, so it gives E0603; is_tuple_type_exported (exports.rs:323) works.
  EVIDENCE: oracle/src/main.rs classify / member_info / call_info; first build error log out/build1.log (E0603 at main.rs:227)
- [high] Join coverage, oxc_parser 0.155 node vs tsrs node with key (start, end) = (oxc span) vs (skip_trivia(pos), end): src/compiler 99.943% of 556,724 nodes (core AOT kinds 99.938% of 408,446); all TypeScript src/ 99.644% of 1,073,338 (core 99.565%); AWFY TS port 99.459% (core 99.758%); AWFY JS 99.494% (core 100%); Preact JS 99.252%. IdentifierReference, StaticMemberExpression, CallExpression, NewExpression, Binary/Logical/Assignment, ChainExpression (matched to the inner node), TSAs/TSNonNull and Parenthesized are all 100%. 0 kind mismatches anywhere. CRLF line endings (TypeScript's sources use CRLF) and multi-byte UTF-8 / astral characters align byte-exactly.
  EVIDENCE: out/ts-compiler.report.json, out/ts-all.report.json, out/awfy-ts.report.json, out/awfy-js.report.json, out/preact.report.json (scripts/join.py); scripts/run.sh pipeline
- [high] Leftover misses are all non-expressions on the tsrs side. ts-all oxc StringLiteral misses: ImportDeclaration 1,527, quoted ObjectProperty key 1,237, ExportAllDeclaration 535, `declare module` 10. `constructor` IdentifierName (tsrs has no name node): 111. Object spread is oxc SpreadElement vs tsrs SpreadAssignment, not walked: 403. Two fixes were needed in the prototype. (1) tsgo's is_part_of_type_node returns true for every NullKeyword (tsrs_ast utilities_2.rs:641-656), so value-position `null` must be kept explicitly. (2) tsrs's walk records names inside type-only declarations (interface members, type params, literal types): 12,279 extra records in src/compiler. Skipping type-only subtrees fixed this.
  EVIDENCE: ts-all no-span analysis (python over out/ts-all.oxc.jsonl); before/after numbers in out/awfy-ts and out/ts-compiler reports
- [high] Span hazard: tsrs decodes file bytes before parsing. It strips a UTF-8 BOM, transcodes UTF-16 LE/BE BOM files to UTF-8, and replaces invalid UTF-8 with U+FFFD. Its offsets are therefore relative to the decoded text. A BOM file joined at 0% until the oxc side stripped the BOM; after that it was 100%. The side table must carry a hash of the exact decoded text, and the AOT side must decode the same way or drop hints for that file.
  EVIDENCE: third_party/tsrs/crates/tsrs_vfs/src/internal/internal.rs:217-236 (decode_bytes); corpus/bom test: 0.0% -> 100.0% after joiner/src/main.rs strips U+FEFF
- [high] In JS files, a JSDoc cast `/** @type {X} */ (e)` adds a tsrs AsExpression with the same span as the inner expression, so one span carries two types (before and after the cast). Join by kind preference (inner PropertyAccess vs AsExpression), or key the cast on the enclosing ParenthesizedExpression. Preact had 3 such nodes.
  EVIDENCE: preact: tsrs AsExpression spans coincide with oxc StaticMemberExpression x2 and ObjectExpression x1 (inspection script over out/preact.jsonl)
- [high] Check speed is not a bottleneck for offline AOT. AWFY TS (40 KB): 12.6 ms check. TypeScript src/compiler (10 MB, 195k lines, 234 files incl. 47 lib + 109 .d.ts): program 50-75 ms, check 0.41-0.50 s single-threaded, 107k types, RSS 119 MiB after check. All TypeScript src/ (18 MB, 455k lines, 602 files): check 0.90-1.0 s single-threaded, 248 MiB after check; with --parallel (16 checkers) check 0.22 s, export 0.58 s, total 0.87 s wall / 4.7 s CPU, peak 827 MiB. Single-checker and 16-checker side tables are identical (0 differing records out of 544,280 and 1,041,946).
  EVIDENCE: stats lines of out/ts-compiler.jsonl, out/par-ts-compiler*.jsonl, out/par-ts-all*.jsonl; /usr/bin/time -l; diff script over par-* outputs
- [high] Export cost: hint-only export of 544k nodes takes 1.2 s and peaks at 223 MiB. Adding type_to_string for every node adds about 1 s and about 620 MiB, because node-builder output goes into arenas that are never freed. Keeping serde_json Value trees in memory added about 1 GB (a prototype artifact). The JSON side table is about 195 bytes per node (106 MB for 10 MB of source; 200 MB for 18 MB). A binary encoding would be needed.
  EVIDENCE: runs with/without --no-typestr: walk 1,207 ms / 223 MiB vs 2,194 ms / 847 MiB; earlier Value-tree version 1,222-1,887 MiB; file sizes in out/
- [high] Hint precision on value nodes, typed TS vs JS. AWFY typed TS port: any 0.2%, precise category 97.1%, member access resolved 100%, field slot found 64.0%, union receivers 0%, call with declared target 97.1%, method-on-class (virtual) calls 60%. The same code as original un-annotated JS with checkJs: any 38.8%, precise 59.9%, member resolved 74.6%, receiver any 24.2%, slot 48.1%, call target 70.7%. Preact (JSDoc-typed JS, 12 files): any 30.4%, precise 44.3%, member resolved 67.2%, slot 0%. All TypeScript src/: any 2.6% (setup errors inflate it), member resolved 98.1%, slot 4.2%, union receivers 3.0%, call target 95.6%.
  EVIDENCE: scripts/precision.py over out/awfy-ts.jsonl, out/awfy-js.jsonl, out/preact.jsonl, out/par-ts-all--parallel.jsonl
- [high] Class layout hints only help class-based code. Receivers of member accesses: AWFY TS = class instance 91% (53% polymorphic this), 67% fields / 33% methods. TypeScript src/compiler = structural interface 47.1%, object 40.4% (mostly namespace/enum objects), union 5.2%, class instance 0.2%. 21,760 of 67,902 (32%) member accesses there are enum members (SyntaxKind.X), which the checker can fold through get_constant_value. TS interfaces and even class types are structural, so a slot index is only a candidate.
  EVIDENCE: receiver-category analysis over out/*.jsonl; samples in out/hint-samples.json (e.g. jBody.mass -> Body slot 6/7; this.v1.walkStrength -> Variable slot 3/6; packet.link -> Packet slot 0/5)
- [high] Flow narrowing is in the table: inside `while (this.currentTask !== null)` the receiver of this.currentTask.link is TaskControlBlock (non-null). Element access gives array element type plus index type (this.taskTable[identity] -> recv array<nullable TaskControlBlock>, index number). A call through a function-typed field resolves only to the FunctionType annotation (richards.ts:414), not to a closure body. TS `number` never separates int from double: `let i = 0; i += 0.5` is just number. ep9 still needs its own range analysis and closure analysis.
  EVIDENCE: out/hint-samples.json; richards.ts lines 392-393, 199, 295; holes H8
- [high] The hints are unsound. 13 small cases type-check with only an unrelated lib error (console), yet the oracle's hint is wrong at runtime in every case (node 24). H1 `const p: Point = {x,y}` gives hint instance Point, runtime instanceof false. H2 arr[5] gives number, runtime undefined. H3 double cast gives string, runtime number. H4 JSON.parse into {n:number} gives number, runtime string. H5 method-param bivariance: TypeError. H6 covariant arrays: TypeError. H7 `maybe!`: TypeError. H8 int vs 0.5. H9 a .d.ts says parseCount(): number, the JS returns string; the call target is flagged dts:true. H10 the .d.ts gives field x slot 0/2, the real construction order is y,x. H11 delete through any: number vs undefined. H12 prototype patched. H13 property narrowing kept after a mutating call: hint Point, runtime null, TypeError.
  EVIDENCE: corpus/holes/src/holes.ts, corpus/holes/src/vendor/fast.{js,d.ts}; `node src/holes.ts` output; out/holes.jsonl hints
- [high] Trust in .d.ts: the checker never reads the JS behind a .d.ts, so every type that comes from a .d.ts is a claim about foreign code. The share of member accesses / calls whose declaration is in a .d.ts or lib: TypeScript src/compiler 8.5% / 8.6%, all src/ 9.4% / 10.1%, AWFY TS 4.9% / 10.9%, Preact 65.1% / 47.4% (its core VNode/Component shapes live in src/internal.d.ts). The prototype tags each resolved declaration with dts / lib, so the AOT compiler can treat those as untrusted boundaries.
  EVIDENCE: precision.py dts% columns; decl_info() in oracle/src/main.rs
- [high] checkJs works for project JS: classes, `this.x =` constructor fields (decl kind BinaryExpression), JSDoc @param/@returns. Plain JS under node_modules is NOT analyzed at the default maxNodeModuleJsDepth 0: TS7016, every hint is error/any. With maxNodeModuleJsDepth 2 tsrs infers makeCounter(): Counter, c.add(2): number with method target, c.value: number at slot 0/2. TS 7 removed moduleResolution node10 (TS5108), so JS projects need nodenext or bundler.
  EVIDENCE: corpus/nm-js runs out/nm-js-0.jsonl vs out/nm-js-2.jsonl; corpus/awfy-js first run diag TS5108
- [high] API visibility: tsrs_checker has 703 pub fns vs 2,499 pub(crate) fns, plus 93 `_exported` wrappers. These are pub(crate) with no wrapper: get_type_of_node, check_expression(_cached), get_type_of_expression, get_narrowed_type, get_iterated_type_or_element_type (needed for for-of element types), get_signatures_of_symbol, get_this_type, is_const_context. get_type_at_location covers most AOT needs. Rule from the docs: types from different checkers must not be mixed (program.rs comment at get_type_checker_for_file). Checker methods take &mut self. Span-keyed facts (category, class decl file:offset, slot, target decl) do not depend on which checker produced them, which is why the parallel export works through Program::for_each_checker_group.
  EVIDENCE: grep counts over crates/tsrs_checker/src; third_party/tsrs/crates/tsrs_compiler/src/program.rs:1199-1214; docs/CHECKER.md 'Checker' section
- [high] Process-global state and lifetime: new_program leaks a &'static Program. Freeing it needs `unsafe fn free_program` / `free_unshared_program` (program.rs:590-620). Program, checker and type memory goes to thread arenas that live until process exit unless the caller sets up tsrs's region machinery; issue #597 aborted after exhausting the shared 32 GiB range. Compressed pointers are on for every unix build unless the plain-ptrs feature is set (tsrs_core/build.rs). They reserve 32 GiB PROT_NONE at fixed address 0x4001_0000_0000 and abort if the address is taken (reserve.rs:17-20, 47, 83-107). The AST keeps 1M source-text slots that are never reused (tsrs_ast/src/identifier.rs:30-42). Static counts: tsrs_core 77, tsrs_compiler 44, tsrs_checker 38, tsrs_tsoptions 31, tsrs_ast 18; thread_local blocks 14. Checker-pool settings are OnceLock globals (checkerpool.rs:297-1754), and 44 TSRS_* environment variables are read.
  EVIDENCE: files/lines cited; notes/mem-build-free-programs.md lines 1-38; grep counts
- [high] Linking tsrs and oxc_parser 0.155 in one binary forces oxc onto unicode-id-start 1.1.2 (Unicode 15.1) instead of 1.5.0. tsrs pins it with '=1.1.2' in its workspace Cargo.toml. Earlier research showed this makes oxc fail 16 test262 Unicode 16/17 identifier tests.
  EVIDENCE: work/tsrs-type-oracle/combined/Cargo.toml; `cargo tree -i unicode-id-start` -> 'Adding unicode-id-start v1.1.2 (available: v1.5.0)' under oxc_regular_expression/oxc_ast; research/tsrs-conformance.md
- [high] Churn: commits touching crates/tsrs_checker since 2026-10-01: 211 (26 in the last week). exports.rs 10, services.rs 11, types.rs 56, checker.rs 82, compiler program.rs 76, checkerpool.rs 67, core arena.rs 43, ptr.rs 52. The repo has about 198 commits in the last week; it was created 2026-10-01. Practical drift test with the unchanged oracle: against tsrs ec07c2b5 (2026-10-12) it needed only a 2-line fix (bundled::embed was private then). Against da895ac2 (2026-10-05) it had 6 compile errors: positions were i32 before the #306 unsigned refactor, and a host function's arity changed. Most recent commits to the API files are lint, format and refactor work, not features.
  EVIDENCE: gh api repos/oxc-project/tsrs/commits?path=...&since=...; partial clone repos/tsrs-history; out/build-old.log, out/build-old2.log; oracle-old/
- [high] Prior art agrees that TS types cannot be trusted as-is for native speed. Static Hermes (facebook/hermes branch static_h, last commit 2026-10-19; doc marked 'in progress and considered unstable') runs its own sound typed dialect (Flow syntax; TS only partly, through -parse-ts). It has nominal classes, exact objects, bounds-checked typed arrays that are incompatible with untyped JS arrays, checked casts from any, and final methods. Porffor reads TS annotation names syntactically, with no checker, and maps them straight to IR types; optTypes is on by default.
  EVIDENCE: facebook/hermes static_h doc/TypedLanguage.md lines 3, 9, 26, 41, 54, 119, 121, 167 (saved at out/hermes-TypedLanguage.md); repos/CanadaHonk_porffor compiler/codegen.js:2598-2636, 2821-2823, 5428-5437 and compiler/prefs.js:1 (commit deda4945, 2026-10-19); web search: Chain React 2024 slides (speakerdeck.com/tmikov2023/optimizing-with-static-hermes-chain-react-2024) state that sound types are required for performance

## Risks
- The side-table schema hides tsrs churn, but every tsrs bump still needs adapter fixes. tsrs publishes no crates, keeps PRs closed, follows a 'faithful port, improvements are bugs' policy, and gets about 200 commits a week.
- Unsound hints used without guards cause silent wrong behaviour. All 13 holes pass the checker, and a class type does not guarantee an instance or a field order (H1, H10).
- The span join silently breaks if the text ep9 compiles differs from the text tsrs checked (BOM, UTF-16, invalid UTF-8, a file edited between steps). Mitigation: a content hash per file.
- Files with type errors give error/any types (TypeScript src/ with missing @types/mocha: 1.3% error-typed core nodes); hints there must be dropped.
- Interface-heavy and JS-heavy code gets little layout benefit: 0.2% slot hits in src/compiler, 0% in Preact, 30-39% any in JS. The AOT speed-up from TS types may be small outside class-heavy numeric code. Not measured: no codegen was built.
- The tsrs process memory model (fixed 32 GiB reservation, arenas live until exit) caps a single build's size. Issue #597 hit it at about 4,800 composite projects without regions. Very large monorepos may need per-project runs.
- Results are on one 18-core arm64 Mac. tsrs perf on x86 and Linux and the cost with mimalloc were not measured here.
- Two corpus caveats: my DeltaBlue port replaces the SOM Vector.sort with an insertion sort, and the TypeScript src/ run had setup diagnostics (2,482, mostly missing test typings), so its any/error rates are pessimistic.

## Open questions
- What speed-up do guarded hints actually give over baseline JIT feedback? This needs an ep9 AOT prototype; not measured here.
- Should ep9 offer a sound strict mode (Static Hermes style: nominal classes, checked casts, bounds-checked typed arrays)? It changes JS semantics for opted-in code, so it is a product decision.
- Can enough of the hint export be done with the public API to avoid a fork? It worked here, but for-of element types (get_iterated_type_or_element_type) and raw expression types are pub(crate), and an extended AOT pass may want them.
- How large is the checkJs cost and precision on a real node_modules tree with maxNodeModuleJsDepth ≥ 2 (for example webpack or vscode)? Only a toy package was tested.
- Does the fixed 0x4001_0000_0000 tsrs reservation collide with the address ranges ep9's own GC cage design picks? Irrelevant if tsrs stays in a separate binary.
- What binary side-table format, and how is the hint table versioned against both the tsrs version and the ep9 IR version?

## Artifacts
- [artifacts/tsrs-type-oracle/oracle/src/main.rs](artifacts/tsrs-type-oracle/oracle/src/main.rs)
- [artifacts/tsrs-type-oracle/oracle/Cargo.toml](artifacts/tsrs-type-oracle/oracle/Cargo.toml)
- [artifacts/tsrs-type-oracle/joiner/src/main.rs](artifacts/tsrs-type-oracle/joiner/src/main.rs)
- [artifacts/tsrs-type-oracle/scripts/join.py](artifacts/tsrs-type-oracle/scripts/join.py)
- [artifacts/tsrs-type-oracle/scripts/precision.py](artifacts/tsrs-type-oracle/scripts/precision.py)
- [artifacts/tsrs-type-oracle/scripts/run.sh](artifacts/tsrs-type-oracle/scripts/run.sh)
- scratchpad/work/tsrs-type-oracle/corpus/awfy-ts/src/richards.ts
- scratchpad/work/tsrs-type-oracle/corpus/awfy-ts/src/deltablue.ts
- scratchpad/work/tsrs-type-oracle/corpus/awfy-ts/src/nbody.ts
- scratchpad/work/tsrs-type-oracle/corpus/awfy-ts/src/som.ts
- scratchpad/work/tsrs-type-oracle/corpus/holes/src/holes.ts
- scratchpad/work/tsrs-type-oracle/corpus/holes/src/vendor/fast.d.ts
- scratchpad/work/tsrs-type-oracle/out/hint-samples.json
- scratchpad/work/tsrs-type-oracle/out/ts-compiler.report.json
- scratchpad/work/tsrs-type-oracle/out/ts-all.report.json
- scratchpad/work/tsrs-type-oracle/out/awfy-ts.report.json
- scratchpad/work/tsrs-type-oracle/out/preact.report.json
- scratchpad/work/tsrs-type-oracle/out/hermes-TypedLanguage.md
- scratchpad/work/tsrs-type-oracle/out/build-old2.log
- scratchpad/work/tsrs-type-oracle/combined/Cargo.toml
