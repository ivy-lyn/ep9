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
| Units keep source | `ep9_bytecode` unit tests: AOT units have `source`; hash and length match | M0 (H35) |
| eval, Function, toString pass in every mode | test262 runner per mode: 1,805 files call `eval(` or `Function(` outside staging (2,057 with staging); 80 files in `built-ins/Function/prototype/toString` | M1 |
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
| test262 d53ebc8b | 1,805 / 2,057 eval-or-Function files; 80 toString files (grep at the pin, 2026-10-21) | `third_party/test262.commit` |
| P4 baseline | 11 tests pass lone-surrogate source to eval: 3 with a literal (`invalid-lone-surrogate-groupname.js`, 2 staging/sm RegExp), 8 that eval every code unit 0..0xFFFF (S7.4_A5, S7.4_A6, 4 x S7.8.5 regexp, 2 annexB RegExp escape) | `probe::p4_lone_surrogate_eval_count` |

## Validation

- M0: the dependency check runs in CI once `ep9_vm` or `ep9_cli` runs code.
- M1: the test262 runner runs eval, Function and toString tests in every mode.
- M2 exit: P4. Any of the 11 tests that fails through the shim triggers P4 (01 §7).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H05-Q1 | Which crate is "the runtime" for the dependency check: `ep9_vm` or a facade crate | First runnable VM | The crate that exposes `eval`; check `ep9_cli` too |
| H05-Q2 | Size budget for the always-linked front end | M1 (SIZE-REPORT) | Report only; no budget before M1 |
| H05-Q3 | Shim design for lone-surrogate source (re-encode, or a WTF-8 path in oxc) | M2 exit (P4) | Map each lone surrogate to a private marker, map positions back with `LineMap` |

## Reopens if

- A user decision adds a lean mode (N13 changes).
- P4 fails and D1 moves to an own parser: the shim then disappears, the rule stays.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft: rule details A1-A4, checks, test262 counts, P4 baseline 11 | Front-end agent |
| 2026-10-21 | rewrite | A1 (C2, M2-P0): the compiler edge moves from the runtime crate to the binary. `ep9_vm` gets the compiler through the new hook `IsolateConfig::compiler: Option<&'static CompilerHooks>` (`compile_eval`, `compile_function`, `compile_script` -> `CompileResult`), and `ep9_cli` and `ep9_test262` fill it from `ep9_frontend`. The Rules check is dep-guards check 12 (ep9 default and jitless link `ep9_frontend`; `ep9_vm` does not). Nothing calls the hooks in M2-P0 (P3-EVAL-VM, P3-FNCTOR). Pre-freeze churn (README §3.3) | M2-P0 integration fixer |
