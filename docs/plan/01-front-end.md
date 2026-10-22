# ep9 plan: front end and the tsrs answer

**Evidence tags:** [Rnn] = `../research/nn-*.md`. [P1] = plan v1 local probe. [T262], [TSRS] = pinned checkouts. Full table at the end.

- tsrs is **not** the engine parser. oxc parses every line of code that runs (D1 default E).
- tsrs keeps its v1 role: `ep9 check` spawns the tsrs CLI out of process (unix; Windows after SP-tsrs-win).
- tsrs gets a **new** role: the offline AOT type oracle `ep9-typecheck`. It writes a `.ep9h` hint table. It is never linked into the engine.
- TypeScript runtime constructs now lower in BytecodeGen, not in oxc_transformer. This keeps hint keys valid on lowered code (H3).
- `CompiledUnit` becomes a deterministic, versioned ABI with stable function ids and an optional hint section (H1, H2).

IDs used here: D1, D4, D12, D18, D20; H1-H4, H23, H36, H37; P1-P5; L0-L2; A1h, A4; K-A4H; SP-tsrs-win, SP-tsrs, SP-HintPrecision; HINT-SET; M0-M4; RK5-RK8, RK19, RK27, RK38, RK39.

---

## 1. Short answer

```
                             third_party/tsrs  (pinned by tsrs.commit, git-ignored)
                                      |
      +-------------------+-----------+--------------+---------------------------+
      |                   |                          |                           |
 (a) engine parser   (b) `ep9 check`           (c) ep9-typecheck (NEW)      (d) tsrs_scanner
     NO: oxc does it     tsrs CLI, own process     offline AOT type oracle      (~6.5-6.8k lines)
     (D1 = E)            own workspace + lock      separate workspace,          seeds option D,
                         unix; Windows after       links tsrs by path,          only if a P-gate
                         SP-tsrs-win               writes .ep9h (H37)           trips
```

**The gap is by design, not a bug.** TypeScript reports most early errors in the checker, not the parser [R02]. tsrs is a faithful port where "improvements are bugs", and PRs are closed [R01]. A friendly maintainer cannot change this without breaking the port rule [R09]. The same port makes tsrs a good **checker**: its public API is enough to export types [R13].

## 2. Measured facts

### 2.1 tsrs vs oxc as the engine parser

| Measure | tsrs @ f527e984 | oxc 0.155.0 (parser + semantic + regex) | Evidence |
|---|---|---|---|
| test262 parse-negative tests caught (4,653) | 26.8%. With its regex validator 34.5%, but it then rejects 74 valid tests (99.83% of positives) | 100% (4,653/4,653) | [R02], [R09] |
| Valid tests accepted | 99.90% (42 valid programs rejected) | 99.98% (7 Annex B `f() = 1` cases) | [R02] |
| Sloppy mode | Binder is always strict: 868-889 false errors | Full sloppy mode and Annex B | [R04], [R01], [R09] |
| Strict, goal and target state in the parser | None (`const a;` gives no error) | Yes | [R09] |
| `x = y <!-- z` | Silently misparsed | Correct | [R09] |
| JSX and JSDoc in .js | JSX always on; JSDoc rewrites the AST | ep9 sets the source type: no JSX in .js | [R09] |
| Parse speed, 8 real files | 212 MB/s summed, 1.2-3.3x slower | 327 MB/s summed | [R03] |
| Parse + scope analysis | 135 MB/s (parse + bind) | 181 MB/s (parse + semantic) | [R03] |
| Depth before abort, 8 MB stack | About 18.6k nested arrays | About 4.1k nested arrays. A 50k-term `a?b:…` chain aborts in the parser; 200k-term `1+…` and `a.b.…` chains abort in semantic | [R03], [R09], [P1] |
| Process-wide hazards | Fixed 32 GiB reservation at 0x4001_0000_0000 that aborts; 1M source slots never reused; panic on relative file names | none | [R09] |
| Cargo graph leak | `unicode-id-start =1.1.2` downgrades oxc (16 test262 failures) | none | [R02], [R04] |
| Re-parse of one function; eval-context parse options | No public API; none | No public API; none (#11793 open) | [R03], [R04] |
| Windows | Not built, tested or shipped upstream; plain pointers on Windows | Builds on Windows | [TSRS] release.yml, build.rs |
| Upstream | PRs closed; about 200 commits a week | MIT, takes PRs; 60 breaking releases a year | [R01], [R04] |

### 2.2 tsrs as an offline type oracle (prototype)

| Measure | Result | Evidence |
|---|---|---|
| Join, TypeScript `src/compiler` (10 MB) | 99.94% of 556,724 oxc nodes; 0 kind mismatches | [R13] |
| Join, all TypeScript `src/` (18 MB) | 99.64% of 1.07M nodes; 0 kind mismatches | [R13] |
| Leftover misses | Non-expressions on the tsrs side: import specifiers, quoted keys, `constructor`, object spread | [R13] |
| Check + export cost | 0.87 s wall for 18 MB with 16 checkers. Output is byte-identical to 1 checker | [R13] |
| Precision, typed class code (AWFY TS port) | 0.2% `any`; field slot found for 64% of member accesses; 97% of calls have a resolved target | [R13] |
| Precision, idiomatic large TS (`src/compiler`) | Slot found for 0.2%; 47% of receivers are interfaces; 32% of member accesses are enum members | [R13] |
| Precision, plain JS via checkJs | 30-39% `any` (Preact JSDoc 30%, AWFY JS 39%) | [R13] |
| Internal API drift | 2-line fix against an 8-day-old tsrs; 6 compile errors against a 15-day-old tsrs | [R13] |
| Offset hazard | tsrs strips a BOM, transcodes UTF-16 and repairs bad UTF-8. Offsets shift. A BOM file joined at 0% until fixed | [R13] |
| When the join was measured | On the oxc_parser AST, **before** oxc's TS transform. That transform builds nodes with dummy spans | [R17] |
| Unsoundness | 13 crafted hole cases pass the checker, and the hint is wrong at runtime in each. Crafted, so not a rate | [R13], [R17] |
| Build | 21 tsrs crates, 363k lines; 38 s clean build; 13.6 MB stripped binary | [R13] |

## 3. Options

| Option | What ep9 must add | Early errors | Speed | Upkeep | Verdict |
|---|---|---|---|---|---|
| A. tsrs as shipped | Everything in row B | 26.8% | 1.2-3.3x slower than oxc | n/a | Reject |
| B. tsrs fork + ep9 early-error and scope pass | About 3,400 checks (52% local AST, 35% scope or strict context) [R02]. About 8 parser/scanner patches. JSX/JSDoc off. Depth guard. Unicode 17. `plain-ptrs` | 100% is possible | Still slower: TS mode needs 1.4-1.65x oxc's instructions [R03] | Permanent fork against about 200 commits a week | Reject (+6-12 PM and a permanent rebase cost) |
| C. oxc `=0.155.0` behind an ep9 boundary | Depth guard (3 layers, §6), eval context, Function() API. Offset re-entry later | 100% today | Fastest measured | Exact pin, planned bumps, upstream PRs | **Adopt now** |
| D. Own V8-style parser (preparse, re-parse, eval context, UTF-16 input) | About 20k lines, the size of Brimstone's [R04] | Brimstone 99.87%; 100% with oxc as oracle | Best possible startup | Full ownership | Only if a P-gate trips (§7) |
| E. boa_parser | Box AST, no TS | 99.79% | 10-25x slower than oxc | n/a | Reject [R04] |
| F. swc | Own regex validator | 66.7% | About 0.5x oxc | 21 major versions in 12 months | Reject [R04] |
| G. tsrs out of process for `ep9 check` | Nothing in the engine | n/a | n/a | Built in its own workspace | **Adopt** (unix; Windows after SP-tsrs-win) |
| H. tsrs as offline type oracle (`ep9-typecheck`) | A separate workspace binary, the `.ep9h` schema (H37), the join in `ep9_frontend` | n/a | 0.87 s for 18 MB (16 checkers) [R13] | Public API; drift budget ≤ 1 PM/year and ≤ 2 local tsrs patches (est.) | **Adopt** (unix first; A1h) |

## 4. Recommendation (default for D1 = E)

```
oxc now (=pin) ----> local oxc patch set (<= 3 patches) ----> own parser ep9_parser
  upstream PRs first    third_party/oxc.commit                   only if a P-gate trips;
                        + [patch.crates-io]                      seeded by a tsrs_scanner port;
                                                                 oxc stays the differential oracle
tsrs CLI      : `ep9 check` spawns the tsrs binary (own workspace, lock, process; unix until SP-tsrs-win)
ep9-typecheck : `ep9c build --hints` spawns it (separate workspace, links tsrs by path) -> .ep9h (optional)
```

Why:
1. **Correctness now.** Only oxc catches 100% of test262 early errors today [R02], [R04].
2. **Startup speed.** oxc parse + semantic runs at 181 MB/s; tsrs parse + bind at 135 MB/s [R03].
3. **Scope data.** oxc_semantic gives DirectEval, With and Strict scope flags and resolved references (67/67 on the probe) [R03]. The tsrs binder is always strict [R09].
4. **Process safety.** In-process tsrs brings a fixed 32 GiB reservation that aborts. It would compete with ep9's cage [R09], [R13].
5. **Clean dependency graph.** tsrs's exact Unicode pin downgraded oxc in a shared build [R02], [R13].
6. **Upstream access.** oxc takes PRs; tsrs does not. No upstream contact (D12, decided 2026-10-20). Agents make no public posts. oxc gaps become local patches and upstream-ready drafts.
7. **Replaceable.** The boundary keeps oxc swappable. Nova kept raw pointers into the oxc AST and is stuck on oxc 0.124 [R04].

**What would change this answer.** tsrs would need four things: strict, goal and target state in the parser; ECMAScript early errors outside the checker; a plain-JS mode without JSX and JSDoc; no fixed reservation. All four conflict with its faithful-port rule, so a change is unlikely [R01], [R09].

## 5. Front-end boundary and `CompiledUnit` (H1, H2)

```
ep9_frontend   (the ONLY crate that imports oxc_*; it may run on any thread)
  compile_script(src, ScriptOpts)              -> Result<CompiledUnit, SyntaxError>
  compile_module(src, ModuleOpts)              -> Result<CompiledUnit, SyntaxError>
  compile_eval(src, EvalContext)               -> Result<CompiledUnit, SyntaxError>
      EvalContext = { strict, in_function, in_method, in_derived_ctor,
                      in_class_field_init, private_names, new_target_ok }
  compile_function_ctor(params, body, kind)    -> Result<CompiledUnit, SyntaxError>
  compile_lazy(script_id, fn_range, ScopeInfo) -> Result<CompiledFunction, SyntaxError>   (L1, M4)
  Script/ModuleOpts also carry: TS flags (H3) and an optional .ep9h file record (H37)

CompiledUnit  (no GC objects; Rust-owned; serializable; = code-cache format = unit bytecode section)
  header : bytecode ABI version | decoded-source hash | goal | strict | TS flags | engine build id
  functions (stable id = preorder index): bytecode, side tables (handlers, liveness, positions),
           ScopeInfo, flags, source range
  constant pools: indices plus owned literals (WTF-16 strings, f64, BigInt digits, regex source)
  position tables: UTF-8 offsets; the UTF-16 column map is built lazily
  hint section (optional): HINTED entries keyed by (function id, feedback slot); format in ep9_facts

isolate thread:  finalize(CompiledUnit) -> atoms, SharedFunctionInfos, BytecodeArrays on the heap
```

- **Parse path.** Parse, semantic and bytecode generation run on a pooled worker thread. Its stack is sized for the depth cap.
- **Parse options.** ep9 uses `SemanticBuilder::new_compiler()`, `with_check_syntax_error(true)` and `parse_regular_expression = true`. Without regex parsing, oxc catches only 92.3% [R04]. ep9 always sets the source type: script or module, no JSX, TS only for .ts/.mts/.cts.
- **Scope pass.** It reads oxc `Scoping`. It adds closure capture (about 15 lines [R03]), TDZ-check removal, `with`/eval dynamic lookup and register or context slot allocation.
- **Stable function id.** The preorder index of the function in the source tree. Units, profiles and hints use it (H2, H37).
- **Determinism rule (H2).** The same decoded source, flags and ABI version give the same bytes. No hash-map iteration order, pointer values or thread timing may reach the output.
  - The content hash covers functions, pools and tables. It excludes the engine build id.
  - CI compiles the test262 + bench corpus twice per target and compares hashes (M0 exit gate).

## 6. Depth guard, 3 layers (unchanged)

A bracket pre-scan alone is not enough: chains with no brackets build deep trees [R03], [P1].

| Layer | What | When |
|---|---|---|
| 1. Parser | Depth counter in oxc_parser for every recursive production: brackets, conditional, assignment, unary, arrow, `**`, templates, classes. Upstream (#21953, #24375) or a local patch | M1 exit. Until then a bracket pre-scan sizes the stack |
| 2. AST | Iterative pass measures AST depth after parse, before semantic. Above the cap: RangeError. Covers left-deep chains (`1+1+…`, `a.b.b…`, call chains) | M0 |
| 3. ep9 code | Scope pass and BytecodeGen check depth explicitly, or grow the stack (stacker style) | M0 |

**Cap.** Depth above the cap gives a RangeError, as V8 does. The default is 20k levels (est.), above tsrs's 18.6k, and needs about 40 MB of stack. H23 owns the full failure policy ([02-architecture.md](./02-architecture.md)).

## 7. oxc upstream track, local patches, P-gates (unchanged)

Owner: the front-end lead agent (D12). Agents draft; only the user posts upstream ([12-governance §5](./12-governance.md#5-upstream-contact-oxc-and-other-public-projects)).

| Item | oxc reference | Needed by | Interim workaround |
|---|---|---|---|
| Eval-context parse options | #11793 (open since 2025-06) | M1 exit | Script parse (no wrapper) that drops only the allowed nodes' errors, plus post-checks (`eval.rs` E1), then a local patch |
| Depth limit that returns an error, every recursive production | #21953, #24375 | M1 exit | Pre-scan, sized stack, AST-depth pass (M0) |
| Parse Function() parameters and body apart | none | M1 exit | Three parses (`function_ctor.rs` F1): the parameter text and the body text each in a wrapper whose shape is checked, then the joined text through `compile_eval` |
| Strict script without a double parse | none | M2 | Nova's double parse |
| 7 Annex B `f() = 1` positives (runtime error, not early error) | file an issue | M2 | Local patch |
| WTF-8 or UTF-16 source input | none | M3 | Lone-surrogate shim ([07-runtime.md](./07-runtime.md)) |
| Parse a function at an offset with context (hook: `parse_function_body`) | none | M4 | L1 retained arena (§8) |

**Local patches.** They use `third_party/oxc.commit`, `scripts/sync-oxc.sh` and `third_party/patches/oxc/*.patch`, consumed through `[patch.crates-io]`. The pin points at the oxc commit that published 0.155.0. Every patch links its upstream PR. The cap is 3 patches (gate P1).

**P-gates for an own parser (option D).** P1, P2 and P4 are checked at M2 exit; P3 and P5 at M4 exit.

| Gate | Trigger |
|---|---|
| P1 | Eval context or the depth limit is neither upstream nor in a local set of ≤ 3 patches |
| P2 | Measured work for oxc version bumps is > 1 PM per year (est.) |
| P3 | After L1: compiling typescript.js (9.1 MB) takes > 1.5x V8 cold compile, or retained front-end memory is > 2x V8's (est.) |
| P4 | Any test262 lone-surrogate eval case fails through the shim (M0 counts them, incl. S7.4_A5 and S7.4_A6 [T262]) |
| P5 | Parse + compile is > 25% of cold start on the npm corpus (est.) |

Option D costs 6-12 PM. The team ports `tsrs_scanner` into `crates/ep9_parser` as a copy that keeps the NOTICE. It never becomes a path dependency. oxc runs beside it as the differential oracle.

## 8. Lazy-compile policy

| Stage | Policy | When | Evidence |
|---|---|---|---|
| L0 | Compile every function eagerly, then free the arena | M0-M3 | [R04] |
| L1 | Compile the top level only. Keep each script's oxc arena in a per-isolate cache (64 MB start, tune). Heap objects never point to it. On eviction, the next miss re-parses once and compiles every remaining function. Each script re-parses at most once | M4 | [R03]: arena 3-17 B per source byte; re-parse about 370 MB/s |
| L2 | Re-enter the parser at a function offset (oxc upstream) or use a preparser (option D). Add bytecode flushing | After a P-gate or the upstream API | [R04]: V8, SpiderMonkey and JSC all preparse |

- From M0, a SharedFunctionInfo carries its source range, function flags and outer ScopeInfo (H4). So L1 and L2 need no object-model change [R09].
- **AOT rule (new).** `ep9c` compiles every function eagerly at build time, and units carry eager bytecode (H4). L1 and L2 apply only to source loaded at run time: scripts, eval, new Function, modules without a unit.

## 9. Running TypeScript (changed)

**Rule (H3).** BytecodeGen lowers every TS runtime construct from the TS AST. oxc_transformer is not on this path.
- Why: oxc_transformer gives lowered `[[Set]]` fields, parameter properties and enums a dummy span [R17]. Those are the stores where shape and slot hints matter, so the hint key is lost.
- Cost: 0.5-1.5 PM (est.), built in M4. Header fields are fixed in M0 week 3; the ADR lands at M0 exit.

| TS construct | Lowering location | Hint-safe |
|---|---|---|
| Type annotations, interfaces, type aliases, `declare` statements | BytecodeGen skips the node (as Nova does [R04]) | n/a (no code) |
| `as`, `!`, `satisfies`, `<T>x` | BytecodeGen emits the inner expression | Yes; casts demote hints ([05-aot.md](./05-aot.md)) |
| `enum` (isolatedModules form), `namespace` with values, parameter properties, `declare` fields (no field emitted) | BytecodeGen. Enum member values are folded with `ep9_math`; oxc_semantic ConstantValue numbers are not used (H36-deps) | Yes |
| Class fields, `[[Set]]` or `[[Define]]` per module flag | BytecodeGen | Yes |
| Legacy decorators (`experimentalDecorators`, `emitDecoratorMetadata`) | oxc_transformer legacy decorator pass, only in files that use them (D4) | **No**: these files get no hints in v1 |

Any other TS-only runtime form found at M4 also goes into BytecodeGen. The M4 ADR lists them and fixes the pass order for decorator files. Positions stay exact, so no source maps are needed.

**Per-module TS flags (H3, D20).**

| Flag | Source | Default without tsconfig | Stored in |
|---|---|---|---|
| `useDefineForClassFields` | tsconfig, incl. its target-based default | `[[Define]]`, as node type stripping does [R12] | CompiledUnit header, `.ep9h` header, cache key |
| `experimentalDecorators` | tsconfig | off | same |
| `emitDecoratorMetadata` | tsconfig | off | same |

- The same class source gives different object shapes under `[[Set]]` and `[[Define]]`. 4 of 6 measured codebases use `[[Set]]`: a target below ES2022, or vscode's explicit `false` [R12].
- D20 is decided (2026-10-20): follow tsconfig, else `[[Define]]`. The H3 ADR fixes it. The header fields exist from M0 week 3; the lowering is built in M4.

## 10. Hint join (front-end side)

```
 .ts source --> oxc_parser + oxc_semantic --> TS AST (real spans, before lowering)
                                                   |
 .ep9h (ep9-typecheck) --> decoded-text hash ok? --+--> join on (start, end, kind)
                           no: drop the file's hints,       | no record: no hint for that node
                           count it                         v
                                    BytecodeGen (+ TS lowering, H3) writes HINTED entries
                                    keyed by (function id, feedback slot)
                                    -> CompiledUnit hint section (format in ep9_facts)
```

| Rule | Detail | Evidence |
|---|---|---|
| Join point | The oxc AST **before** TS lowering, inside `ep9_frontend` | [R17] |
| Key | oxc span (start, end) + node kind. The tsrs side uses (skip_trivia(pos), end) | [R01], [R13] |
| File check | Hash of the decoded text. ep9 strips a UTF-8 BOM before offsets and hashing, as tsrs does | [R13] |
| JSDoc casts | Key on the ParenthesizedExpression span; a JSDoc cast shares its span with the inner node | [R13] |
| Mismatch | A hash or kind mismatch drops every hint of that file. ep9c counts and reports it | [R13] |
| Output | BytecodeGen writes HINTED entries keyed by (function id, feedback slot). The schema is in [05-aot.md](./05-aot.md) | [R16] |
| Files with type errors | ep9-typecheck marks them; their hints are dropped | [R13] |
| Bundled or minified input | `ep9c` builds the module graph from original sources, so its output keeps hints. Pre-bundled or minified input from other tools gets no hints. A source-map join is post-v1 | [R17] |

**Gate and use.** A1h gate: the join matches ≥ 99.5% of core node kinds (est.), and SP-HintPrecision reports precision on HINT-SET. No codegen reads hints before A4 (D18 default b). K-A4H decides whether HINTED consumers stay ([05-aot.md](./05-aot.md)).

## 11. ep9-typecheck (summary)

`ep9-typecheck` lives in `tools/ep9-typecheck`, a separate Cargo workspace. It links tsrs crates by path from `third_party/tsrs/crates/*`, with tsrs's Cargo.lock and toolchain 1.99.1. Its only ep9 dependency is `ep9_hints`, which has zero dependencies. `ep9c build --hints` spawns it. It is built in A1h (1-2 PM, AOT lane, after M1 exit, due M4 exit), unix first. Hints are optional in every mode: JIT mode needs none, and plain JS uses PROVEN and PROFILED facts only. Pipeline, queries and schema: [05-aot.md](./05-aot.md). Build and sync: [08-repo.md](./08-repo.md).

## 12. If option B is ever chosen

| Item | Content |
|---|---|
| Mandatory conditions [R09] | A `patches/` step in the sync script. An engine-owned early-error and scope pass that never uses the tsrs binder. A depth guard. `plain-ptrs` instead of the fixed reservation. Patches for Unicode 17, JSX/JSDoc off, sloppy octal, HTML comments and static `import source` |
| Extra cost | +6-12 PM on the front end, plus a permanent rebase cost |
| SP-tsrs (optional, 0.25-0.5 PM) | Apply the minimal patch set. Replay 4 weeks of upstream commits. Count conflicts and hours |
| Oracle | `ep9-typecheck` stays a separate binary, unless the fork uses `plain-ptrs` and Unicode 17. Only then can engine and oracle share one tsrs build |
| Hash check | Stays in either case. Two parse paths can still read different bytes (BOM, UTF-16, edits) |

**Front-end risks** ([10-quality.md](./10-quality.md)): RK5-RK8 (oxc churn and gaps, tsrs in the engine, pin leak), RK19 (lone surrogates), RK27 (tsrs on Windows), RK38 (oracle churn), RK39 (span join).

## Changes vs plan v1

| Area | v1 | v2 |
|---|---|---|
| tsrs roles | Not the parser; `ep9 check` only | Adds role (c): offline AOT type oracle `ep9-typecheck`; new option row H |
| TS execution (v1 §2.9) | oxc_transformer for enums, namespaces, parameter properties | BytecodeGen lowers them; oxc_transformer only for legacy decorators (no hints) |
| TS flags | not recorded | Per module in header, `.ep9h` and cache key (H3, D20) |
| CompiledUnit | GC-free, serializable | + ABI version, decoded-source hash, stable function ids, hint section, determinism rule (H1, H2) |
| Other | n/a | New hint-join and bundling rules (§10); lazy policy gains the AOT eager rule (§8); option B notes oracle consequences (§12) |
| Unchanged | | Measured facts, A-G verdicts, engine parser (oxc, D1 = E), depth guard, oxc track, patch cap, P-gates, L0-L2 |
| Decisions 2026-10-20 | | D12: no upstream contact; agents draft, only the user posts. D20 decided. Enum folds use `ep9_math` (H36). See [decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction) |

## Evidence tags

| Tag | Path |
|---|---|
| [R01] | [../research/01-tsrs-architecture.md](../research/01-tsrs-architecture.md) |
| [R02] | [../research/02-tsrs-conformance.md](../research/02-tsrs-conformance.md) |
| [R03] | [../research/03-parser-perf-and-features.md](../research/03-parser-perf-and-features.md) |
| [R04] | [../research/04-parser-alternatives.md](../research/04-parser-alternatives.md) |
| [R09] | [../research/09-critic.md](../research/09-critic.md) |
| [R12] | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R13] | [../research/13-tsrs-type-oracle.md](../research/13-tsrs-type-oracle.md) |
| [R16] | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R17] | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) (wins over [R16]) |
| [P1] | Plan v1 local probe (session-local); re-checked by M0 week 3 depth tests |
| [T262] | `third_party/test262` at `test262.commit` |
| [TSRS] | `third_party/tsrs` at `tsrs.commit` |
