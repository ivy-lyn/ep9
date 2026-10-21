# H01: Front-end boundary

| Field | Value |
|---|---|
| Status | Draft (2026-10-21): F1 built in `ep9_frontend` (depth guard, worker, scope pass v0, positions) |
| Fix by | M0 wk3 |
| Freeze point | M1 exit |
| Origin | v1#1; plan status E |
| Crates | `ep9_frontend` (the only oxc importer); users: `ep9_bytecode` (CompiledUnit), `ep9_vm` (`finalize`), `ep9_unit` |
| Owner (plan role) | Front-end lead agent (D12) |
| Validated by | `ep9_frontend` tests (rules below); test262 parse gate; determinism CI (M0 exit 12) |
| Plan sources | [02 §6 H1](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [01 §4-§7](../plan/01-front-end.md#5-front-end-boundary-and-compiledunit-h1-h2), [H02](H02-bytecode-abi.md), [H23 row](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37) |

## Context

- D1 = E: oxc =0.155.0 now, at most 3 local patches, an own parser only on a P-gate. The boundary keeps that swap possible.
- Nova kept raw pointers into the oxc AST and is stuck on oxc 0.124 [R04]. An AST costs 5.8-6.8x the source size [R01], so it must not live past compilation.
- oxc has no depth limit before M1 (layer 1). A deep input overflows the stack and aborts the process. The boundary must turn that into a RangeError.

## Decision

Copied from the plan row: the AST never leaves `ep9_frontend`. CompiledUnit is GC-free and serializable; it is the code cache and the unit bytecode format. `finalize()` runs on the isolate thread. The header adds bytecode ABI version, decoded-source hash, TS flags, stable function ids and an optional hint section.

F1 fills these gaps (kind **fill**):

| # | Decision | Why |
|---|---|---|
| B1 | Public API: `check_early_errors(src, SourceOpts)` and `analyze(src, SourceOpts) -> ScopeAnalysis`. Errors are `CompileError::{Syntax(EarlyErrors), Range(RangeError)}`. `SourceOpts` sets goal and strictness explicitly; a sloppy module cannot exist | Plain values cross the boundary; no oxc type |
| B2 | Layer 0 (token pre-scan) runs on the caller. Everything else runs on a persistent per-thread worker with a sized stack (`stack.rs`: 2 MiB + 1.5 x max(25,000 x parse unit, 20,000 x pass level); virtual, about 167 MiB debug, 94 MiB release). A failed thread spawn is a RangeError | One parse never aborts the process |
| B3 | Depth caps: `AST_CAP` = 20,000 AST levels (layer 2 and layer 3), `PRESCAN_CAP` = 25,000 units (layer 0). Above a cap: RangeError with a position | 01 §6; above V8's limit for every bracket, prefix and arrow form (node 24.22: 1,064 arrows to 12,392 unary operators), not for flat binary chains (see Costs) |
| B4 | The scope pass resolves names on its own scope tree (ECMA-262 environments). oxc reference and scope ids are only `pub(crate)` lookup keys for BytecodeGen | The oxc tree differs from the spec in 4 places (scope/mod.rs table) |
| B5 | `ScopeAnalysis` is plain Rust data with u32 ids, built in source order. Hash maps are only for lookup, never iterated | H2 determinism |
| B6 | Positions are UTF-8 byte offsets. `LineMap` gives line and UTF-16 column, also for WTF-8 lone surrogates. It agrees with `ep9_bytecode::SourceLines` on every offset of valid UTF-8 | 01 §5: offsets stored, columns lazy |
| B7 | Stable function id = preorder index. When the walk enters a class body (after the heritage), each synthetic class function gets its id, followed at once by the functions inside it: the default constructor (only if the class has no `constructor`); the instance initializer (only if there is an instance field, accessor or private method), then the functions in the instance field values; the static initializer (only if there is a static field, accessor or static block), then the functions in the static field values and static blocks; then the methods and the functions in computed keys and decorators, in element order. So the ids are a preorder of the parent links (the bytecode verifier Tree rule). As built: `crates/ep9_frontend/src/scope/collect.rs:1207-1211`, the `FunctionId` doc `crates/ep9_frontend/src/scope/mod.rs:76-80` (M2-P2-CLSCG-FNID-PREORDER) | H2 needs one rule for functions without source |

## Rules

| Rule | Check | From |
|---|---|---|
| Only `ep9_frontend` lists an `oxc_*` dependency | `grep oxc crates/*/Cargo.toml` (one hit); review | M0 |
| No `pub` item names an oxc type | `tests::no_public_item_names_an_oxc_type` | F1 |
| Every nesting form: cap compiles, cap + 1 is a RangeError, debug and release | `depth_tests::m0_*`, `m1_*`, `x_*` (39 forms + 10 regex forms, `crates/ep9_frontend/src/forms.rs` `FORMS`; `every_form_is_gated`) | F1 |
| No abort on deep input | `depth_tests::a_million_levels_never_abort`, `random_mixes_near_the_caps_never_abort` | F1 |
| The scope pass handles every form at the cap on the worker stack | `depth_tests::check_ast_form` runs `analyze` at the cap | F1 |
| Layer 3 reports a RangeError, not an abort | `scope::tests::layer_3_reports_a_range_error` | F1 |
| Equal input gives an equal `ScopeAnalysis` | `scope::tests::equal_input_gives_equal_output`; corpus run compares 2 runs per file | F1 |
| Every oxc reference and scope has an ep9 counterpart | `probe::scope_pass_matches_oxc_on_corpus` (0 missing) | F1 |
| Positions match the CompiledUnit reader | `position::tests::agrees_with_the_compiled_unit_source_lines` | F1 |

## Consequences

- **Gains.** oxc can change or be replaced behind one crate. Deep input fails with a RangeError on every target. The scope pass follows the spec where the oxc tree does not; it takes 0.40 s for 49,517 test262 + bench files (4.6 M references, release, aarch64).
- **Costs.** About 167 MiB (debug) / 94 MiB (release) of virtual stack per thread that compiles. Binary and logical chains deeper than 20,000 are a RangeError; V8 accepts about 4 M. Own resolution is code that oxc already has.
- **Follow-ups.** Layer 1 (oxc depth counter) at M1. Eval: `compile_eval` with an `EvalContext` is built (M0 month 2 P2, `crates/ep9_frontend/src/eval.rs`) as a wrapper parse plus post-checks; the oxc eval-context option (oxc #11793) or a local patch is due at M1 exit ([01 §7](../plan/01-front-end.md#7-oxc-upstream-track-local-patches-p-gates-unchanged)). BytecodeGen consumes `ScopeAnalysis` (built in M0 wk3-4).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R04] | Only oxc + semantic catches 100% of parse-phase early errors; Nova is stuck on oxc 0.124 | [../research/04-parser-alternatives.md](../research/04-parser-alternatives.md) |
| [R01] | AST region 5.8-6.8x the source | [../research/01-tsrs-architecture.md](../research/01-tsrs-architecture.md) |
| [R09] | Verdict on the parser path | [../research/09-critic.md](../research/09-critic.md) |
| F1 corpus | 546 references resolve unlike oxc; all are the 3 explained classes (split body 54, Annex B 488, catch var 4) | `probe::scope_pass_matches_oxc_on_corpus` |

## Validation

- `cargo test -p ep9_frontend` (debug) runs in the CI build-test job (`cargo test --workspace`) on macOS arm64, Linux x64, Linux arm64 and Windows x64. The release run (`--release`) is local only: no CI step runs it. x86_64 macOS runs only locally under Rosetta. `cargo run --release -p ep9_test262 -- parse` stays green (CI test262 job, 4 targets).
- M0 exit 7 (nesting forms at cap + 1 give RangeError) and 12 (identical bytecode hashes).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H01-Q1 | `oxc_ast_visit` is pinned in `ep9_frontend/Cargo.toml`, not in root `workspace.dependencies` | Next bump PR | **Closed 2026-10-21 (fill):** the root list pins `oxc_ast_visit = "=0.155.0"`; `ep9_frontend` uses `workspace = true`. `Cargo.lock` did not change |
| H01-Q2 | Two line maps exist (`LineMap`, `SourceLines`) | BytecodeGen PR | Keep one, WTF-8 aware, in `ep9_bytecode` |
| H01-Q3 | The synthetic class function id rule (B7) needs the H2 owner's agreement | H2 Accepted | B7 as written |
| H01-Q4 | Binary chains above 20,000 terms: RangeError or an iterative path | M1 | Iterative chains in BytecodeGen, then a higher cap for chains only |
| H01-Q5 | Annex B: two same-name functions in one block hoist (V8, SpiderMonkey), `arguments` block functions do not (spec, main test262 test) | M1 test262 run | As built; 2 staging/sm tests stay red |

## Reopens if

- P1 or P2 fails (D1 changes the parser path), or layer 1 cannot land in ≤ 3 patches.
- The corpus cross-check finds a difference outside the explained classes.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from the F1 build: API, worker, caps, scope pass, positions, function ids | Front-end agent |
| 2026-10-21 | fill | H01-Q1 closed: `oxc_ast_visit` pin moved to the root `workspace.dependencies` | docs-sync agent |
| 2026-10-22 | rewrite | B7 follows the code (M2-P2-CLSCG-FNID-PREORDER, 5083e3d): each synthetic class function is followed at once by the functions inside it (default constructor; instance initializer, then the instance field values; static initializer, then the static field values and static blocks; then methods and computed keys in element order). The old text gave all synthetic ids before any element, which the bytecode verifier Tree rule rejects once fields compile. A Decision row changes, so the kind is rewrite | P3-ACCEPT-2 |
| 2026-10-22 | rewrite | Rules row "Every nesting form": 39 forms + 10 regex forms (was "+ 2"). 34bd740 (2026-10-21, pre-scan: v-flag classes that stay open after a nested class) added 8 regex forms to `FORMS` with no ADR row. The new forms fail on the old pre-scan (a worker stack overflow at 100k levels of v-flag classes, not a RangeError), so the check changes result | P3-ACCEPT-2 |
| 2026-10-22 | clarify | Follow-ups: `compile_eval` with an `EvalContext` is built (M0 month 2 P2, wrapper parse); the oxc option or a local patch stays due at M1 exit (01 §7). BytecodeGen consumes `ScopeAnalysis` (done). No check changes | P3-ACCEPT-2 |
| 2026-10-22 | clarify | Validation says where each run happens: CI runs the debug tests on the 4 CI targets (Windows x64 runs them, not only a compile check); the release run and x86_64 macOS are local only. No check changes | P3-ACCEPT-2 |
