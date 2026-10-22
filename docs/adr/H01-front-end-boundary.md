# H01: Front-end boundary

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P4-ACCEPT; review record complete). Was: Draft (2026-10-21): F1 built in `ep9_frontend` (depth guard, worker, scope pass v0, positions) |
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
| B7 | Stable function id = preorder index. When the walk enters a class body (after the heritage), each synthetic class function gets its id, followed at once by the functions inside it: the default constructor (only if the class has no `constructor`); the instance initializer (only if there is an instance field, accessor or private method), then the functions in the instance field values; the static initializer (only if there is a static field, accessor or static block), then the functions in the static field values and static blocks; then the methods and the functions in computed keys and decorators, in element order. So the ids are a preorder of the parent links (the bytecode verifier Tree rule). As built: `crates/ep9_frontend/src/scope/collect.rs:1208-1212`, the `FunctionId` doc `crates/ep9_frontend/src/scope/mod.rs:76-80` (M2-P2-CLSCG-FNID-PREORDER) | H2 needs one rule for functions without source |

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
- **Follow-ups.** Layer 1 (oxc depth counter) at M1. Eval: `compile_eval` with an `EvalContext` is built (M0 month 2 P2, `crates/ep9_frontend/src/eval.rs`). It parses the eval text as a Script, with no wrapper (a wrapper accepts `return 1`; decision E1, `crates/ep9_frontend/src/eval.rs:30`). It drops a recoverable oxc error only when the error is the context-dependent error of an allowed node (`new.target`, `super`, `#x`) and its label span is that node's, then runs post-checks. The oxc eval-context option (oxc #11793) or a local patch is due at M1 exit ([01 §7](../plan/01-front-end.md#7-oxc-upstream-track-local-patches-p-gates-unchanged)). BytecodeGen consumes `ScopeAnalysis` (built in M0 wk3-4).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R04] | Only oxc + semantic catches 100% of parse-phase early errors; Nova is stuck on oxc 0.124 | [../research/04-parser-alternatives.md](../research/04-parser-alternatives.md) |
| [R01] | AST region 5.8-6.8x the source | [../research/01-tsrs-architecture.md](../research/01-tsrs-architecture.md) |
| [R09] | Verdict on the parser path | [../research/09-critic.md](../research/09-critic.md) |
| F1 corpus | 546 references resolve unlike oxc; all are the 3 explained classes (split body 54, Annex B 488, catch var 4) | `probe::scope_pass_matches_oxc_on_corpus` |

## Validation

- `cargo test -p ep9_frontend` (debug) runs in the CI build-test job (`cargo test --workspace`) on macOS arm64, Linux x64, Linux arm64 and Windows x64. In release, CI runs only the corpus determinism test (`bytecodegen::corpus`, CI test262 job, 4 targets; M0 exit 12); the release run of the other tests is local only. x86_64 macOS runs only locally under Rosetta. `cargo run --release -p ep9_test262 -- parse` stays green (CI test262 job, 4 targets).
- M0 exit 7 (nesting forms at cap + 1 give RangeError) and 12 (identical bytecode hashes).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H01-Q1 | `oxc_ast_visit` is pinned in `ep9_frontend/Cargo.toml`, not in root `workspace.dependencies` | Next bump PR | **Closed 2026-10-21 (fill):** the root list pins `oxc_ast_visit = "=0.155.0"`; `ep9_frontend` uses `workspace = true`. `Cargo.lock` did not change |
| H01-Q2 | Two line maps exist (`LineMap`, `SourceLines`). Both are in use: `LineMap` in the front end (`crates/ep9_frontend/src/position.rs:43`), `SourceLines` in the disassembler and the VM (`crates/ep9_bytecode/src/positions.rs:140`). Joining them is a refactor that no M0 exit item needs | M1 (was "BytecodeGen PR"; M2-P3-OQ-H01-Q2) | Keep one, WTF-8 aware, in `ep9_bytecode` |
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
| 2026-10-22 | clarify | H01-Q2 stays open: Needed by moves from "BytecodeGen PR" to M1. `LineMap` and `SourceLines` are both in use, and joining them is a refactor that no M0 exit item needs. No check changes (M2-P3-OQ-H01-Q2) | P3 integration fixer |
| 2026-10-22 | clarify | Follow-ups: eval parses its text as a Script, with no wrapper (`eval.rs` decision E1), and drops only the context-dependent errors of allowed nodes by label span. The 2026-10-22 Follow-ups row above said "wrapper parse", which the code never did (Codex C1, M2-P4-B2-ROW2). No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | Validation: in release, CI runs the corpus determinism test (`bytecodegen::corpus`) on 4 targets; only the release run of the other tests is local. No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | 1 `path:line` reference moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P4-ACCEPT). The §3.1 review record is complete: row 1 P3-ACCEPT-2, row 2 Codex (M2-P3-B2-ROW2 and the P4 pre-step run (M2-P4-B2-ROW2)), row 3 P4-ACCEPT-3 (and the P4 review checkers for the RF1 rows), row 4 data. The orchestrator did not write the rows it approves. No check changes | orchestrator |
| 2026-10-22 | clarify | 1 `.github/workflows/ci.yml` line reference moved down 6 lines after the setup-node step (M2-P4-CI-NODE24; drift script, [I15]). No check changes. Accepted-path change (README §3.2: orchestrator + review record) | orchestrator |
| 2026-10-23 | clarify (proposed; Accepted path) | 2 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it; the orchestrator approves | M3-P0 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P3-ACCEPT-2 (did not write the ADR) | Decision = the 02 §6 H1 row, word for word, plus the F1 fills B1-B7. Rules checked against `ep9_frontend` at e719ffa: only `crates/ep9_frontend/Cargo.toml` lists an `oxc_*` dependency (the root pins them); every named test exists and passes; `FORMS` has 49 forms (39 + 10 regex); `AST_CAP` 20,000 and `PRESCAN_CAP` 25,000 (`crates/ep9_frontend/src/depth.rs:29`, :37); the worker stack (`stack.rs`) is 2 MiB + 1.5 x max(25,000 x 4,608 B debug or 2,560 B release, 20,000 x the pass level), about 167 MiB debug and 94 MiB release. R1: B7 was not the as-built order of synthetic class function ids (fixed, rewrite). `ep9 --print-bytecode` on `class A { static s = () => 1; m() {} x = () => 2; [(() => 'k')()] = 3; static { (() => 4)(); } }` gives: f1 default constructor, f2 instance initializer, f3 x's arrow, f4 static initializer, f5 s's arrow, f6 the static-block arrow, f7 m, f8 the computed-key arrow, as the new B7 says. R2: Rules row 3 said 2 regex forms; 34bd740 made it 10 (fixed, rewrite). R3: Follow-ups were stale: `compile_eval` is built (fixed). R4: Validation said Windows x64 is a compile check and listed x86_64 macOS; CI runs the debug tests on 4 targets, the release run is local only (fixed). Open and due in M0, not filled (reported): H01-Q2 (two line maps: `LineMap` and `ep9_bytecode::SourceLines` both still exist; BytecodeGen is built) and H01-Q3 (B7 needs the H2 owner's agreement; H2 is accepted after ABI 5). Change-log kinds: agree | c68971a |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | Two runs. (1) M2-P3-B2-ROW2: the "ADR docs" slice of the P3 merge (115 KB, 18 ADRs, base 66f08cd): no finding on H01 (its 2 findings are in H30 and H32). (2) The P4 pre-step run on every P3 change of H01, H06, H09, H10, H11, H28 and H36, fc368e1 and 077d1fe included (base f8746a4 = 938d2bc without those changes): verdict needs-attention, 3 findings. On H01, C1 (medium): Follow-ups and the P3-ACCEPT-2 Follow-ups row say eval is a wrapper parse; `crates/ep9_frontend/src/eval.rs` parses the text as a Script and filters diagnostics (E1) | decisions-log M2-P3-B2-ROW2; M2-P4-B2-ROW2; Codex text outside the repo: scratchpad review/m2p3-doc.codex.txt, review/m2p4-b2.codex.txt |
| 3 Verifier | P4-ACCEPT-3 (wrote none of the rows it checks) | Checked at 0c4c70e (tag m2-p3 938d2bc + one decisions-log commit). C1 reproduced: `on_worker` parses with `SourceType::script()` and keeps every diagnostic that `Allowed::drops` does not match (`crates/ep9_frontend/src/eval.rs:150-179`); the test `script_only_errors_stay_errors` keeps `return 1` a SyntaxError. Real, category (b) (made by the P3-ACCEPT-2 Follow-ups edit). Fixed (clarify row). V1: Validation said no CI step runs the release tests, but CI runs the release corpus test on 4 targets (`.github/workflows/ci.yml:203`) (fixed, clarify). Rows written by others: the B7 rewrite (`crates/ep9_frontend/src/scope/collect.rs:1208-1212` and `crates/ep9_frontend/src/scope/mod.rs:76-80` give the stated order: true; kind rewrite: agree); the "Every nesting form" rewrite (`FORMS` has 49 entries, 10 of them `regex-*`: true; rewrite: agree); the Follow-ups clarify (C1); the Validation clarify (V1); the fc368e1 H01-Q2 clarify (`LineMap`, `crates/ep9_frontend/src/position.rs:43`, is used in 5 front-end files; `SourceLines`, `crates/ep9_bytecode/src/positions.rs:140`, by the disassembler and `crates/ep9_vm/src/unit.rs`; Needed by M1 as M2-P3-OQ decided; clarify: agree, as the H05-Q3 Needed-by row). Open, not due in M0: H01-Q3 (H2 is Accepted after ABI 5). Change-log kinds: agree | branch m0m2/p4-accept-3; local run, mac a64, 0c4c70e |
| 4 Data | from the "Validated by" line | `cargo test -p ep9_frontend --locked`: 199 pass, 7 ignored, debug and release (with the 49 depth forms at the caps). `probe::scope_pass_matches_oxc_on_corpus` (release, `--ignored`): 4,602,576 references, 0 with no ep9 counterpart, 0 different on a second run, 546 resolved unlike oxc (split body 54, Annex B 488, catch var 4), scope pass 408 ms. `cargo run --release -p ep9_test262 -- parse`: positives 47,357 / 47,364, negatives 4,653 / 4,653, 0 new failures. The Linux and Windows rows are the CI build-test and test262 jobs. P4 refresh (0c4c70e): `golden_disassembly`, `throwing_instructions_have_positions` and `every_form_is_gated` pass (debug). M0 exit item 12 (was listed missing): the real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3); its test262 jobs run the release corpus digest step on 4 targets and the debug one on linux-x64, against BCG-CORPUS-9 `ee7dca40…` (`expectations/bytecode-corpus.digest`) | local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/frontend.log, frontend-release.log, probe-scope.log, t262-parse.log; P4: scratchpad m0m2/accept3/logs/frontend.log |
| 1 Reviewer (amendment 2026-10-22, M2-P4-CI-NODE24 refs) | reviewer agent (did not write the change) | The moved `.github/workflows/ci.yml` refs point at the same text as before the 6-line setup-node step (compared HEAD~1 old lines with the new lines); nothing else changed but the refs and one Change-log row; kind clarify: agree. Approve (README §3.2: the orchestrator wrote the change, so a second agent approves) | M2-P4-CI-NODE24 |
| Approval (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 approver: a second reviewer agent (README §3.2); it wrote none of the M3-P0 rows | **Not approved**: the review record is not complete. Checked at 70d79e5 (m0m3/p0-fix-docs). Content: true. The 2 refs are the only text change in this ADR besides the Change-log row (`git diff --word-diff 6b652e1..70d79e5`), and each new ref names the item its text names: the synthetic-function preorder comment in `scope/collect.rs` (B7 and row 3). ADRLINE: 0 FIX, 0 UNFIXABLE. Kind clarify: agree. Missing: rows 1-3, which the Change-log row names (README §3.1); none is written. No content change is needed. Approve again once the rows are written, or once the orchestrator logs that a ref-only move needs only one reviewer row (as the 2026-10-22 H28 approvals had) | local read, 70d79e5; scratchpad approve-p0/refpairs.py; ADRLINE (scratchpad p4docs/path_line_drift.py) |
