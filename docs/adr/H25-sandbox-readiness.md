# H25: Sandbox readiness

| Field | Value |
|---|---|
| Status | Draft (2026-10-21): the rule and an inventory of today's off-heap pointers. `ExternalRef<T>` is not built yet (H25-Q1) |
| Fix by | M0 |
| Freeze point | M1 exit. D6 names M1a (object layout freeze) as the last cheap point for a sandbox |
| Origin | v1#23; plan status U |
| Crates | `ep9_vm` (heap cell layouts), `ep9_heap`, `ep9_unit` (unit trust), `ep9_bytecode` (verifier) |
| Owner (plan role) | GC lead (S4, D6); VM lead (cell layouts) |
| Validated by | The native-word tests and the inventory check (Rules); the verifier; the H35 load checks |
| Plan sources | [02 §6 H25](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [11 D6](../plan/11-decisions.md), [03 §4 S4](../plan/03-gc.md), [H12](H12-no-address-identity.md), [H35](H35-aot-unit-format.md) |

## Context

- D6 (decided 2026-10-20): no untrusted-code sandbox in v1. A V8-style sandbox (pointer tables, cage-relative ArrayBuffer pointers, CFI) would cost 6-12 PM and must come before the M1a layout freeze. The `ExternalRef` rule applies from M0 so that the option stays open (11 D6).
- A V8-style sandbox bans raw off-heap pointers inside heap objects. Retrofitting that means touching every layout [R05].
- Depends on: D6, H8 and H17 (layouts), H12 (no address escapes), H35 (unit trust). Depended on by: H13 (snapshot rewrites each off-heap word), H22 (SAB backing stores).

## Decision

Copied from the plan row: off-heap pointers only in `ExternalRef<T>`. Units are trusted code (D6) (02 §6).

| Part | Rule |
|---|---|
| Off-heap pointers | A heap cell holds an off-heap address only in a field of type `ExternalRef<T>`: one typed wrapper, one encoding, one place to change when a sandbox comes |
| Encoding today | The native word of H14: `addr + 2^49`, a subnormal double that is never a cell, so the GC skips it (`crates/ep9_vm/src/frame.rs`) |
| Units | Native unit code is trusted, like a native addon (D6). The loader checks the header, versions and content hash (H35 S4), never the machine code. With a sandbox, only signed or bytecode-only units may load (11 D6) |
| Bytecode | The VM runs only verified bytecode, from any source (compiler, code cache or unit) |

**Inventory of off-heap pointers in heap cells (55c15d0; checked again at 0c4c70e).** `native_of` makes every native word; these are its stores into heap cells.

| Cell | Field | Points at | Store |
|---|---|---|---|
| FUNCTION | +24 | `FunctionInfo` (in its unit, or a host or stub info) | `crates/ep9_vm/src/object.rs:703` |
| FEEDBACK | info word (`fv::INFO`) | `FunctionInfo` | `crates/ep9_vm/src/feedback.rs:169` |
| CONTEXT | scope word (OBJECT_LAYOUT 2, C5) | `ContextScope { unit, scope }` | `crates/ep9_vm/src/object.rs:916` (P0-VM, M2-P0) |

The other `native_of` stores write VM stack frame slots (caller fp, return pc, function info; `crates/ep9_vm/src/call.rs`). The VM stack is not a heap cell.

## Rules

| Rule | Check | From |
|---|---|---|
| A native word is a valid non-cell Value, so the GC never traces an off-heap pointer | `frame::tests::native_words_are_valid_non_cell_values` | M0 wk3 |
| Every store of an off-heap pointer into a heap cell is in the inventory above | `grep -rn 'native_of(' crates/*/src` lists only the inventory rows, the VM stack slots of `call.rs` and the `frame.rs` test (review, until the H25-Q2 lint) | M0 |
| From M1a, every inventory field has the type `ExternalRef<T>` | a lint that rejects `native_of` outside `ExternalRef` (H25-Q2) | M1a |
| No raw address escapes to JS, hashes or embedders | H12 rules | M0 wk4 |
| The VM loads only verified bytecode | `LoadedUnit::load` takes a `VerifiedUnit` (type check); `verifier::*` tests | M0 wk3 |
| A unit's native code loads only after the H35 header, version and hash checks; the machine code is not inspected (trusted, D6) | `ep9_unit` check and mutation tests (`crates/ep9_unit/tests/check.rs`) | M0 wk4 |
| Off-heap data of later kinds (ArrayBuffer and SAB backing stores, external strings, kinds 6-7) uses `ExternalRef<T>` from its first commit | review, then the H25-Q2 lint | M1 |

## Consequences

- **Gains.** A sandbox, if D6 ever changes, edits one type and its loads, not every layout. A snapshot (H13) finds every off-heap word through one type.
- **Costs.** One more wrapper in the VM. Until the lint exists, the inventory relies on review.
- **Follow-ups.** H25-Q1 and H25-Q2 before M1a. The ArrayBuffer and external-string layouts (M1) follow the rule.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | S4 options include a V8-style sandbox; decide early, because it bans raw off-heap pointers inside heap objects | [../research/05-gc-design.md](../research/05-gc-design.md) |
| [R09] | The threat model was never decided; untrusted code in an embedding implies a sandbox, W^X and CFI on JIT entries; about 45% of V8 CVEs were JIT-related | [../research/09-critic.md](../research/09-critic.md) |
| inventory | `grep -rn 'native_of(' crates/*/src` at 55c15d0: object.rs:608, feedback.rs:169, call.rs:92-101 (3 frame slots), frame.rs:220 (test). At 0c4c70e: the FUNCTION, FEEDBACK and CONTEXT stores of the table, the 3 frame slots of `call.rs` and the `frame.rs` test, nothing else (GENERATOR and slot objects hold Values only) | this ADR |

## Validation

- M0: the native-word test and the inventory grep. M1a: the lint passes and every inventory field is an `ExternalRef<T>`.
- If D6 changes: a sandbox build swaps the encoding to a table index and runs test262 and the GC-stress matrix.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H25-Q1 | The shape of `ExternalRef<T>` | M1a | A `#[repr(transparent)]` wrapper of the native-word encoding with typed `new` and `get`; a sandbox build would make it an index into a per-isolate pointer table |
| H25-Q2 | How is the rule checked? | M1a | An `xtask` lint (like gc-lint) that rejects `native_of` and `native_ptr` outside `ExternalRef` and the VM stack code |
| H25-Q3 | Do host function pointers in a `FunctionInfo` need the wrapper? | M1a | No: a `FunctionInfo` is off-heap Rust memory, not a heap cell. Only the cell field that points at it does |

## Reopens if

- D6 changes to a sandbox (an owner request), or an embedder needs to run untrusted code.
- A unit signature scheme is required (H35-Q2).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from 02 §6 row H25 and 11 D6, with the inventory of off-heap pointers in heap cells at 55c15d0 | P0-DOCS agent (M2-P0) |
| 2026-10-21 | clarify | 1 `path:line` reference moved to the code after M2-P0 (drift script, [I15]) | M2-P0 integration fixer |
| 2026-10-22 | clarify | Inventory: the CONTEXT row names its store (`object.rs`, the scope word of `new_context`), and the inventory and its Evidence row record the grep at 0c4c70e: the same 3 heap-cell stores, the 3 frame slots and the test. No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | 4 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P4-ACCEPT-3 (did not write the ADR) | Decision = the 02 §6 H25 row, word for word ("Off-heap pointers only in `ExternalRef<T>`. Units are trusted code (D6)"). Checked at 0c4c70e: `grep -rn 'native_of(' crates/*/src` gives `crates/ep9_vm/src/object.rs:703` (FUNCTION info), `crates/ep9_vm/src/feedback.rs:169` (FEEDBACK info), `crates/ep9_vm/src/object.rs:916` (CONTEXT scope), `crates/ep9_vm/src/call.rs:93-102` (3 frame slots) and the `frame.rs` test, so Rules row 2 holds; the GENERATOR fields and its REGS cell hold Values only (`object::genf`), and slot objects hold Values only (M2-P3-SLOTS-VALUES); a native word is the address + 2^49 (`native`, `DOUBLE_ENCODE_OFFSET`); `LoadedUnit::load` takes a `VerifiedUnit`; `crates/ep9_unit/tests/check.rs` exists. R1: the CONTEXT row had no store ref and the inventory was dated 55c15d0 only (fixed, clarify). `ExternalRef<T>` is not built (H25-Q1, M1a), as the Status line says. Open questions due in M0: none (Q1-Q3 M1a). Change-log kinds: agree | branch m0m2/p4-accept-3 |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | (filled after the P4 merge) | |
| 3 Verifier | P5-ACCEPT-4 | (filled in P5) | |
| 4 Data | from the "Validated by" line | `frame::tests::native_words_are_valid_non_cell_values` passes; the inventory grep (above); `cargo test -p ep9_bytecode --locked` 75 pass (the verifier tests); `cargo test -p ep9_unit --locked` 28 pass (check and mutation tests). The real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3). Missing: the H25-Q2 lint (M1a) | local run, mac a64, 0c4c70e; logs outside the repo: scratchpad m0m2/accept3/logs/vm.log, bytecode.log, unit.log |
