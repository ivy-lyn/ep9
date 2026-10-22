# H05: Compiler, interpreter and source always present

| Field | Value |
|---|---|
| Status | Draft (2026-10-21): the rule, with the checks it needs. Since M2-P0, `ep9` (ep9_cli) links ep9_frontend in every mode (dep-guards check 12) and gives it to the VM through `IsolateConfig::compiler` |
| Fix by | M0 rule (wk3) |
| Freeze point | M1 exit |
| Origin | R16-D9; plan status N |
| Crates | `ep9_frontend` (bytecode compiler), `ep9_vm` (T0 interpreter, eval, `Function`), `ep9_bytecode` (unit source section), `ep9_unit`, `ep9_cli` |
| Owner (plan role) | AOT senior (rule); front-end lead agent (compiler side); VM lead (runtime side) |
| Validated by | test262 runs in every mode (M1); a dependency check in CI; unit format tests |
| Plan sources | [02 §6 H5](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [02 N13](../plan/02-architecture.md), [01 §5](../plan/01-front-end.md#5-front-end-boundary-and-compiledunit-h1-h2), [H01](H01-front-end-boundary.md), [H35](H35-aot-unit-format.md) |

## Context

- ECMAScript makes source text a run-time input: `eval`, `new Function`, `GeneratorFunction` and `AsyncFunction` compile strings, and `Function.prototype.toString` returns source text.
- An AOT-only or "lean" build could drop the compiler and the interpreter to save size. N13 rules that out in full-ECMAScript mode.
- H1 keeps the compiler behind one crate (`ep9_frontend`), so linking it is one dependency edge.

## Decision

Copied from the plan row: every build and mode links the bytecode compiler and T0 and keeps the source. `eval`, `new Function` and `Function.prototype.toString` work in every mode. There is no lean runtime in full-ECMAScript mode.

The rule needs these details (kind **fill**):

| # | Decision | Why |
|---|---|---|
| A1 | The binary that runs code (`ep9_cli`, also `ep9_test262`) depends on `ep9_frontend` as a normal, non-optional dependency. No Cargo feature removes it. `ep9_vm` has no edge to `ep9_frontend` (C2): it reaches the compiler through `IsolateConfig::compiler: Option<&'static CompilerHooks>` (`compile_eval`, `compile_function`, `compile_script`; `crates/ep9_vm/src/compiler.rs`), which the embedder fills from `ep9_frontend` | One edge to check per binary; a feature flag would make a lean build possible. The VM crate stays free of the front end, and eval and `Function()` still find a compiler in every mode |
| A2 | Every unit keeps the decoded source text. AOT units always embed it (`CompiledUnit::source`, H35); the header holds its hash and length | `toString` and lazy compile need it |
| A3 | Each function records its source range: the function node, or the class body for synthetic class functions (`FunctionInfo::span`, H01 B7) | `toString` of a class constructor returns the class text |
| A4 | Source text that holds a lone surrogate (only possible through `eval` and `Function` of a JS string) goes through the lone-surrogate shim; P4 tracks it | oxc reads UTF-8 only |

## Rules

| Rule | Check | From |
|---|---|---|
| A runtime crate never has an optional `ep9_frontend` dependency or a feature that removes it; `ep9_vm` has no `ep9_frontend` edge | CI `dep-guards.sh` check 12: `cargo tree -p ep9_cli [--no-default-features] -e normal -i ep9_frontend` exits 0 and its stdout has an `ep9_cli` node (default and jitless ep9); controls: `-p ep9_vm -e normal,build -i ep9_frontend` matches no package, and `ep9_builtins` (a dev-only edge) does not pass the same query | First VM crate that runs code (M0); check 12 since M2-P0 |
| Units keep source | `ep9_bytecode` test `determinism::source_hash_is_sha256_of_the_decoded_text` (hash and length of the decoded text); AOT units have `source` from A2 (H35) | M0 (H35) |
| eval, Function, toString pass in every mode | test262 runner per mode: 1,829 files call `eval(` or `Function(` outside staging (2,081 with staging); 80 files in `built-ins/Function/prototype/toString` | M1 |
| The lone-surrogate eval set is counted | `probe::p4_lone_surrogate_eval_count` (baseline 11, below) | M0 count; P4 at M2 exit |

## Consequences

- **Gains.** One semantics in every mode: no test262 split between "full" and "lean" builds. A function can always fall back to bytecode (deopt target, H2).
- **Costs.** The compiler and the interpreter are in every binary. Hermes's compiler is 2.4 of its 6.0 MB [R14]. Units are larger by their source text.
- **Follow-ups.** SIZE-REPORT measures the front end's share of the binary at M1. L1 (M4) keeps the source for lazy compile anyway.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R14] | Dual-mode systems keep a compiler for eval; Hermes compiler size | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R17] | The rule rests on test262, not on a speed claim | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) |
| test262 d53ebc8b | 1,829 / 2,081 eval-or-Function files (`grep -rlE 'eval\(\|Function\(' test --include='*.js'` in the test262 checkout, without and with `test/staging/`; 17 of them are `_FIXTURE` files); 80 toString files. Re-counted 2026-10-22: the draft's 1,805 / 2,057 (2026-10-21) did not record its command, and 02 §6 H5 says 1,791 | `third_party/test262.commit` |
| P4 baseline | 11 tests pass lone-surrogate source to eval: 3 with a literal (`invalid-lone-surrogate-groupname.js`, 2 staging/sm RegExp), 8 that eval every code unit 0..0xFFFF (S7.4_A5, S7.4_A6, 4 x S7.8.5 regexp, 2 annexB RegExp escape) | `probe::p4_lone_surrogate_eval_count` |

## Validation

- M0: the dependency check runs in CI: dep-guards check 12 (since M2-P0), in the dep-guards job.
- M1: the test262 runner runs eval, Function and toString tests in every mode.
- M2 exit: P4. Any of the 11 tests that fails through the shim triggers P4 (01 §7).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H05-Q1 | Which crate is "the runtime" for the dependency check: `ep9_vm` or a facade crate | First runnable VM | **Filled 2026-10-22:** the binary crate that fills `IsolateConfig::compiler`: `ep9_cli` (the `ep9` binary) now, `ep9_runtime` when that crate exists. Never `ep9_vm`: it reaches the compiler only through the hook (A1, C2). The check is dep-guards check 12 (Rules row 1) |
| H05-Q2 | Size budget for the always-linked front end | M1 (SIZE-REPORT) | Report only; no budget before M1 |
| H05-Q3 | Shim design for lone-surrogate source (re-encode, or a WTF-8 path in oxc) | M3 (milestone): 01 §7 puts WTF-8 source input at M3, and M3 exit item 6 needs every lone-surrogate eval case to pass ([09 §2.2](../plan/09-milestones.md#22-scope-and-exit-criteria)). The P-gate P4 is still checked at milestone M2 exit (01 §7) | Map each lone surrogate to a private marker, map positions back with `LineMap` |

## Reopens if

- A user decision adds a lean mode (N13 changes).
- P4 fails and D1 moves to an own parser: the shim then disappears, the rule stays.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft: rule details A1-A4, checks, test262 counts, P4 baseline 11 | Front-end agent |
| 2026-10-21 | rewrite | A1 (C2, M2-P0): the compiler edge moves from the runtime crate to the binary. `ep9_vm` gets the compiler through the new hook `IsolateConfig::compiler: Option<&'static CompilerHooks>` (`compile_eval`, `compile_function`, `compile_script` -> `CompileResult`), and `ep9_cli` and `ep9_test262` fill it from `ep9_frontend`. The Rules check is dep-guards check 12 (ep9 default and jitless link `ep9_frontend`; `ep9_vm` does not). Nothing calls the hooks in M2-P0 (P3-EVAL-VM, P3-FNCTOR). Pre-freeze churn (README §3.3) | M2-P0 integration fixer |
| 2026-10-22 | fill | H05-Q1 filled: "the runtime" for the dependency check is the binary that fills `IsolateConfig::compiler`, `ep9_cli` now and `ep9_runtime` later, never `ep9_vm`; the check is dep-guards check 12 (Rules row 1, unchanged). H05-Q3 stays open (D8) | P2-EVAL-FE |
| 2026-10-22 | clarify | H05-Q3 Needed by: M3 (milestone), was "M2 exit (P4)". The shim is milestone-M3 work (01 §7 WTF-8 row; M3 exit item 6; plan-v4 decision D8); "P4" is the parser P-gate, checked at milestone M2 exit, not a month-2 phase. The question stays open. No check changes | P3-ACCEPT-2 |
| 2026-10-22 | clarify | Rules row "Units keep source" names its test (`source_hash_is_sha256_of_the_decoded_text`), and AOT units start at A2; Validation says that dep-guards check 12 runs in CI since M2-P0. No check changes | P3-ACCEPT-2 |
| 2026-10-22 | clarify | test262 counts re-counted at the pin with a written command: 1,829 / 2,081 eval-or-Function files (was 1,805 / 2,057, command not recorded; 02 §6 H5 says 1,791, a plan fix is reported). The 80 toString files are unchanged. No check changes | P3-ACCEPT-2 |
| 2026-10-22 | clarify | 2 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P3-ACCEPT-2 (did not write the ADR) | Decision = the 02 §6 H5 row plus A1-A4 (A1 rewritten in M2-P0 for C2; Q1 filled by P2-EVAL-FE). Checked at e719ffa against dep-guards check 12 (`.github/scripts/dep-guards.sh:191-213`, both controls), `CompilerHooks` and `IsolateConfig::compiler` (`crates/ep9_vm/src/compiler.rs`), the unit source fields (`source_hash`, `source_len`), the span of synthetic class functions (A3: `ep9 --print-bytecode` gives the default constructor and both initializers of a test class the range of its body, 8..96), and `probe::p4_lone_surrogate_eval_count`. R1: H05-Q3 Needed by said "M2 exit (P4)"; the shim is milestone-M3 work (01 §7, M3 exit item 6, plan-v4 D8) (fixed, clarify). R2: the test262 counts (1,805 / 2,057) could not be reproduced and 02 §6 H5 says 1,791; re-counted with a written command: 1,829 / 2,081 (fixed; the 02 row needs a plan fix, reported). R3: Rules row 2 and Validation named no built check (fixed). Change-log kinds: agree (the M2-P0 row is a rewrite: a Decision row changed; the Q1 row is a fill). H05 is verified in P4 (plan-v5 §11, B1 table) | c68971a |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | Not yet run on H05's P3 changes (open, orchestrator). M2-P3-B2-ROW2 counts H05, but the P3 "ADR docs" slice (base 66f08cd) holds 18 ADRs and not H05: `git diff --stat 66f08cd 6146ec5 -- docs/adr` lists no H05, while `git diff e719ffa 938d2bc -- docs/adr/H05-always-linked-compiler.md` has the P3-ACCEPT-2 edits (22 lines). The P4 pre-step run (base f8746a4) covers H01, H06, H09, H10, H11, H28 and H36 only. A Codex run on that H05 diff is needed before Accept | decisions-log M2-P3-B2-ROW2; M2-P4-B2-ROW2; scratchpad review/m2p3-files.txt (H05 is in the P3 file list), review/m2p3-doc.codex.txt |
| 3 Verifier | P4-ACCEPT-3 (wrote none of the rows it checks) | Checked at 0c4c70e (tag m2-p3 938d2bc + one decisions-log commit). No Codex finding to sort (row 2). Rows written by others: the P3-ACCEPT-2 H05-Q3 clarify (01 §7 puts WTF-8 source input at M3, `docs/plan/01-front-end.md:159`; M3 exit item 6 is "All lone-surrogate eval cases pass", `docs/plan/09-milestones.md:123`: true); the test262 recount (re-run at the pin d53ebc8b: 1,829 and 2,081 files, 17 `_FIXTURE` files, 80 toString files: true); the Rules and Validation clarify (`determinism::source_hash_is_sha256_of_the_decoded_text`, `crates/ep9_bytecode/tests/determinism.rs:71`, passes; dep-guards check 12 runs in the CI dependency-guards job: true). A1 as built since P3: `ep9_cli` (`COMPILER`, `crates/ep9_cli/src/run.rs:211`) and `ep9_test262` (`tools/test262/src/exec.rs:532`) fill `IsolateConfig::compiler` and have a normal `ep9_frontend` dependency; eval and `Function()` reach the compiler only through the hook (`crates/ep9_vm/src/abi/script.rs`): true. Change-log kinds: agree | branch m0m2/p4-accept-3; local run, mac a64, 0c4c70e |
| 4 Data | from the "Validated by" line | `bash .github/scripts/dep-guards.sh` check 12: 4 PASS (2 controls; the default and the jitless `ep9` link `ep9_frontend`). `determinism::source_hash_is_sha256_of_the_decoded_text` passes (`cargo test -p ep9_bytecode`, 75 pass). `probe::p4_lone_surrogate_eval_count` (release, `--ignored`): 11, plus 5 files listed for a review by hand. P4 refresh (0c4c70e): dep-guards 29 PASS, check 12 4 PASS; `cargo test -p ep9_bytecode --locked` 75 pass, 2 ignored. Eval and `Function()` in the VM since P3 (EVAL-VM, FNCTOR), test262 at 938d2bc (mac a64, the P3 check run): of the 1,812 eval-or-`Function(` files outside staging (the 1,829 without the 17 fixtures), 1,004 pass and 808 fail; with staging 1,158 of 2,064 pass. The largest fail causes: Annex B functions in sloppy eval 224 (E7, ABI 5), assertion failures 129, `$262.createRealm` 85, generator functions 84, async functions 79. toString: 11 of 80 pass (69 fail: JSON 38, async 11, generators 10, Proxy 6). The real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3). Missing: test262 per mode (M1) | local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/depguards.log, bytecode.log, probe-p4.log; P4: scratchpad m0m2/accept3/logs/bytecode.log, depguards.log, accept3-evalfn.txt; scratchpad m0m2/p3fix/exp/mac/t262-check.json |
