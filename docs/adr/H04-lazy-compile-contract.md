# H04: Lazy-compile contract

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). The SFI fields exist in v0; every function compiles eagerly (L0) |
| Fix by | M0 |
| Freeze point | M1 exit |
| Origin | v1#20; plan status U |
| Crates | `ep9_bytecode` (function source range, outer scope, flags), `ep9_frontend` (compile entry points), `ep9_vm` (FunctionInfo, dispatch slot, call paths) |
| Owner (plan role) | Lead/front end (compile side); VM lead (FunctionInfo side) |
| Validated by | `ep9_bytecode` verifier tests; the T0 call paths; L1 tests at M4 |
| Plan sources | [02 §6 H4](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [01 §5](../plan/01-front-end.md#5-front-end-boundary-and-compiledunit-h1-h2), [01 §8](../plan/01-front-end.md#8-lazy-compile-policy), [06 §2](../plan/06-object-model-and-spec.md#2-header-and-objects-h8-extended) |

## Context

- The lazy-compile policy moves from L0 (compile every function, M0-M3) to L1 (top level only, re-parse once, M4) and maybe L2 (re-enter the parser at an offset, plus bytecode flushing) (01 §8).
- The policy must change without an object-model change. So the SFI must hold, from M0, everything a later compile of one function needs.
- Depends on: H1 (the AST never leaves `ep9_frontend`), H2 (stable function id = preorder index; bytecode offset = deopt id), H5 (source always kept), H14 (dispatch slots).
- Depended on by: H24 (kept ScopeInfo), H35 (units carry eager bytecode), H37 (hints keyed by function id).

## Decision

Copied from the plan row: the SFI holds optional flushable bytecode, the source range, ScopeInfo and context flags. Units carry eager bytecode (02 §6). `ep9c` compiles every function at build time; L1 and L2 apply only to source loaded at run time: scripts, eval, `new Function`, modules without a unit (01 §8).

**As built (v0, M0).** The v0 SFI is `FunctionInfo` (`crates/ep9_vm/src/function_info.rs`). It reaches the compile inputs of its function through its unit and its function id:

```text
FunctionInfo { unit, function_id, flags, code, slot, ... }
  unit.functions[function_id]   source_start..source_end   UTF-8 range in the decoded source
                                outer_scope                the scope that creates the closure
                                kind, flags                strict, generator, async, simple params
  unit.scopes[*]                kind, flags (NEEDS_CONTEXT, CALLS_SLOPPY_EVAL, STRICT), slot names
  unit.source                   the decoded source text (H05 A2)
```

| Plan item | v0 field | Note |
|---|---|---|
| Source range | `Function::source_start`, `source_end` (`crates/ep9_bytecode/src/unit.rs`) | The verifier checks `start <= end <= source_len` |
| ScopeInfo | `Function::outer_scope` + `CompiledUnit::scopes` | Its doc comment names H4 |
| Context flags | `ScopeFlags` on each scope; `FunctionFlags` on each function | Part of the bytecode ABI (H2) |
| Bytecode | `FunctionInfo::code` points into the unit; the dispatch slot entry is the same address | Not optional in v0: L0 compiles all |

## Rules

| Rule | Check | From |
|---|---|---|
| Every function records a source range inside the decoded source, and a valid outer scope | `verifier::function_metadata_rules` (`SourceRange`, `ScopeId` errors) | M0 wk3 |
| Every function of a unit has bytecode (units carry eager bytecode) | `verifier::function_metadata_rules` (`EmptyBytecode`) | M0 wk3 |
| The VM reads compile inputs only from the unit (indices, ranges), never from an AST: `ep9_vm` has no edge to oxc or `ep9_frontend` | `cargo tree -p ep9_vm -e normal,build --locked --target all \| grep -cE 'oxc_\|ep9_frontend'` prints 0 | M0 |
| The unit keeps the source text a later compile needs | H05 rule "units keep source"; `CompileOptions::embed_source` defaults to true | M0 wk3 |
| A call runs bytecode only through an `Interp` dispatch slot. Every call path checks the slot kind first, so a function without bytecode can get its own slot kind later with no change in callers | The T0 fast call (`crates/ep9_vm/src/interp/ops.rs:290`), `slow::call` (`runs_bytecode`), `call::call_function_object` and the generator resume path `call::run_resumed` (M0 month 2 P1) check the kind; `dispatch::tests::*` | M0 wk3 |
| A function compiled lazily gets the same bytecode as its eager compile in the same unit | L1 differential test (H04-Q2) | M4 (L1) |

## Consequences

- **Gains.** L1 and L2 need no FunctionInfo or object-layout change: the range, the outer scope and the flags are already in every unit. Units and code caches keep one format (H1): eager bytecode for all functions.
- **Costs.** L0 compiles functions that never run. The unit (and its source) stays in memory for the isolate's life (`Runtime::units` is never shrunk), so v0 has no bytecode flushing.
- **Follow-ups.** L1 (M4): a "not compiled" dispatch state and the lazy compile entry `compile_lazy(script_id, fn_range, ScopeInfo)` (01 §5). L2: bytecode flushing needs units that can drop a function's bytecode (H04-Q3).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R09] | Lazy compile and bytecode flushing need a policy (keep the AST arena, about 4.6-6.8x the source, or re-parse whole scripts), and source text must stay for `toString` and eval; it constrains early design | [../research/09-critic.md](../research/09-critic.md) |
| [R03] | oxc re-parse runs at about 370 MB/s; the arena costs 3-17 B per source byte (L1 sizing) | [../research/03-parser-perf-and-features.md](../research/03-parser-perf-and-features.md) |
| [R04] | V8, SpiderMonkey and JSC all preparse (L2) | [../research/04-parser-alternatives.md](../research/04-parser-alternatives.md) |

## Validation

- M0: the verifier tests and the dependency check above; every T0 call path checks the slot kind.
- M4 (L1): test262 and the benches with L1 on and off give the same results; the H04-Q2 differential test.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H04-Q1 | How does a function with no bytecode look to the call path? | M4 (L1) | A dispatch slot of a new "lazy" state (kind byte not `Interp`); the cold call path compiles the function, writes `code` and sets the slot to `Interp` (one slot write, H14) |
| H04-Q2 | Must a lazily compiled function get the same bytes as the eager compile of the same source? | M4 (L1) | Yes. Profiles, hints and deopt data key by (function id, bytecode offset or feedback slot) (H2, H37). A CI test compiles each corpus file eagerly and lazily and compares each function's bytes |
| H04-Q3 | Where does flushable bytecode live, given that units own all bytecode today? | L2 | A per-function bytecode cell or allocation that the unit can drop; the dispatch slot goes back to the lazy state |
| H04-Q4 | Is the context-flag set complete for re-parse (for example class field initializers, `arguments` use, direct eval in parameters)? | M4 (L1) | Add flags at the ABI 5 bump (M3-P0) or later bumps (H2), not as a side table. ABI 4 added no scope flag. The ABI 5 list has the `ScopeFlags` bit PARAMETER_ENV (1 << 3) for a direct eval in parameters (M2-P3-ABI5-LIST); class field initializers and `arguments` use have no flag on it |

## Reopens if

- P2 or P3 fires (01 §7): an own parser (option D) can change what a re-parse needs.
- L1 measures re-parse or retained memory above the P3 limits.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from 02 §6 row H4 and 01 §8, with the v0 fields and checks as built at 55c15d0 | P0-DOCS agent (M2-P0) |
| 2026-10-21 | clarify | 1 `path:line` reference moved to the code after M2-P0 (drift script, [I15]) | M2-P0 integration fixer |
| 2026-10-22 | clarify | Rules row "A call runs bytecode only through an `Interp` dispatch slot": the check cell also names the generator resume path `call::run_resumed` (M0 month 2 P1), which checks the kind as the other paths do. No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | H04-Q4 proposed default: ABI 4 shipped with no new scope flag, so the default names ABI 5 (M3-P0), whose list has the PARAMETER_ENV bit for a direct eval in parameters (M2-P3-ABI5-LIST). The question stays open (M4). No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | 2 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P4-ACCEPT-3 (did not write the ADR) | Decision = the 02 §6 H4 row, word for word ("SFI holds optional flushable bytecode, source range, ScopeInfo and context flags. Units carry eager bytecode"), plus the v0 table. Checked at 0c4c70e: `Function::source_start`, `source_end` and `outer_scope` (its doc names H4) in `ep9_bytecode` unit.rs; `ScopeFlags` NEEDS_CONTEXT, CALLS_SLOPPY_EVAL, STRICT (bits 0-2, `ALL` = 0x07) and `FunctionFlags` STRICT, GENERATOR, ASYNC, SIMPLE_PARAMETERS; the verifier errors `SourceRange` and `EmptyBytecode` (`function_metadata_rules`); `CompileOptions::embed_source` defaults to true; `cargo tree -p ep9_vm -e normal,build --locked --target all` has 0 `oxc_` or `ep9_frontend` lines; every frame push that runs bytecode checks the kind first (`crates/ep9_vm/src/interp/ops.rs:290`, `slow::call` `runs_bytecode`, `call::call_function_object`, `call::run_resumed`; the spike-native entry checks it too; `call_host` and `push_construct_stub` run no callee bytecode). R1: the Rules check cell did not name `call::run_resumed`, added in M0 month 2 P1 (fixed, clarify). R2: H04-Q4 named the ABI 4 bump; ABI 4 is closed with no new scope flag and PARAMETER_ENV is on the ABI 5 list (fixed, clarify). Open questions due in M0: none (Q1, Q2, Q4 M4; Q3 L2). Change-log kinds: agree | branch m0m2/p4-accept-3 |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | The P4 docs run (slice m2p4-docs: the P4 edits of 12 ADRs, among them every B3 record and review fix in H04, H23, H24, H25 and H26; base 2e6940a = the P4 head 575569f without the slice): needs-attention, 1 finding, D1 on H09 and H10; no finding on H04. The ADRLINE ref moves of 375d953 came after the slice and were not in it | Codex text outside the repo: scratchpad review/m2p4-docs.codex.txt (file list review/m2p4-docs.list) |
| 3 Verifier | P5-ACCEPT-4 | (filled in P5) | |
| 4 Data | from the "Validated by" line | `cargo test -p ep9_bytecode --locked`: 75 pass, 2 ignored (with `function_metadata_rules`, `position_rules`); `cargo test -p ep9_vm --locked --lib dispatch::tests`: 3 pass; the dependency check prints 0. The real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3). Missing: L1 tests (M4) | local run, mac a64, 0c4c70e; logs outside the repo: scratchpad m0m2/accept3/logs/bytecode.log, vm.log, run-tests.out |
