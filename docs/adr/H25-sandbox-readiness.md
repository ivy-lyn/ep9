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

**Inventory of off-heap pointers in heap cells (55c15d0).** `native_of` makes every native word; these are its stores into heap cells.

| Cell | Field | Points at | Store |
|---|---|---|---|
| FUNCTION | +24 | `FunctionInfo` (in its unit, or a host or stub info) | `crates/ep9_vm/src/object.rs:687` |
| FEEDBACK | info word (`fv::INFO`) | `FunctionInfo` | `crates/ep9_vm/src/feedback.rs:169` |
| CONTEXT | scope word (OBJECT_LAYOUT 2, C5) | `ContextScope { unit, scope }` | P0-VM (M2-P0) |

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
| inventory | `grep -rn 'native_of(' crates/*/src` at 55c15d0: object.rs:608, feedback.rs:169, call.rs:92-101 (3 frame slots), frame.rs:220 (test) | this ADR |

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
