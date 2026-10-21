# H13: Snapshot readiness and stable indices

| Field | Value |
|---|---|
| Status | Draft (2026-10-21): the rules. The index lists are due at M1; the snapshot is built in M4 (A1) |
| Fix by | Rules M0; index lists M1; built M4 (A1) |
| Freeze point | Freeze table (README §3.4): rules and index lists at M1 exit; snapshot build at M4 exit |
| Origin | v1#19 + R16-D5; plan status E |
| Crates | `ep9_vm` (roots, atoms, intrinsics, builtin ids, symbols), `ep9_builtins` (builtin ids), `ep9_bytecode` (runtime ids, constant pools), `ep9_unit` (link table), `ep9_heap` (snapshot layout, M4) |
| Owner (plan role) | VM lead (index lists); runtime lead (A1 snapshot, a CORE item in M4); AOT senior (A0 part 1) |
| Validated by | Golden and order tests per list (Rules); determinism tests; the A1 snapshot gates (M4) |
| Plan sources | [02 §6 H13](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [03 §5-§6](../plan/03-gc.md#6-snapshot-interaction), [06 §2.1](../plan/06-object-model-and-spec.md#21-shapeid-rule-h8), [07 §2-§3](../plan/07-runtime.md#3-a1-code-cache-snapshot-and-bytecode-units) |

## Context

- A startup snapshot (A1, M4), the code cache and AOT units must reach heap objects with no redesign. A snapshot maps, deserializes and rewrites pointers; units then reach its objects only by index or through link cells (03 §6).
- Depends on: H2 (bytecode ABI), H8 (ShapeId rule), H12 (no address identity), H22 (realms), H25 (off-heap pointers), H31 (fuses).
- Depended on by: H35 (link table, RootIndex relocations), H34 (RootIndex kind), H30 (RootRef, IntrinsicRef), H16 and H37 (facts name builtins and intrinsics by index).

## Decision

Copied from the plan row: an external-reference table. Intrinsics in fixed order. Stable indices for isolate roots, realm intrinsics, builtins, atoms, fuses and snapshot root shapes. A deterministic snapshot layout. No raw pointers in bytecode (02 §6).

```text
 build:  snapshot blob { roots[n], realm intrinsics[j], atoms[k], fuses[f], root shapes[s] }
 start:  map -> deserialize -> rewrite pointers -> fill the isolate root table -> load units
 unit:   text --RootIndex(n)--> isolate root table --> object
         text --LinkSlot(i)---> RW link cell (resolved at load or first run) --> object
```

**Index lists at M0 (as built, 55c15d0; counts and the native table re-checked at 383a317, M0 month 2 P1).**

| List | Where | Order check today |
|---|---|---|
| Atoms (`atoms!`, `Atom::ALL`) | `crates/ep9_vm/src/atoms.rs` | `tests::strings::the_atoms_list_is_append_only` |
| Realm intrinsics (`IntrinsicId::ALL`, 33 at 55c15d0, 39 at 383a317) | `crates/ep9_vm/src/builtins.rs` | the IntrinsicId lines of `crates/ep9_vm/tests/golden/vm-builtin-ids.txt` |
| Builtins (`BuiltinId` = block + index; VM block 0, `ep9_builtins` blocks 1-11; 10 and 11 stay empty until P4-PROMISE and P5-B-REFLECT-GLOBALS) | `natives.rs` (VM table), `crates/ep9_builtins/src/*` | `builtin_ids::vm_builtin_ids_match_the_golden`, `table::ids_match_the_goldens`, and the swap tests of both crates |
| Runtime functions (`RuntimeId`) | `crates/ep9_bytecode/src/feedback.rs` | the ABI fingerprint (`runtime_table_text`): `abi::abi_version_matches_history` |
| Well-known symbols (`WellKnownSymbol::ALL`) | `crates/ep9_vm/src/symbol.rs` | `tests::symbols::well_known_symbols_are_isolate_roots_in_order` |
| Isolate roots (`Root`, 32) | `crates/ep9_vm/src/runtime.rs` | count only: `runtime::tests::atoms_and_roots_are_distinct` (H13-Q2) |
| Native runtime-function table (`RuntimeTable`, feature `spike-native`) | `crates/ep9_vm/src/native.rs` (built by P1-NATIVE-ENTRY) | the 7 `RT_*` offsets in declaration order: `tests::native::layout_offsets_read_the_live_isolate` (spike-native builds only); no golden yet (H13-Q3) |
| Fuses, snapshot root shapes, external references | not built (H31 in M1; A1 in M4) | none yet |

## Rules

| Rule | Check | From |
|---|---|---|
| Each index list is append-only: no id is reordered, removed or reused | the order checks in the table above (golden files; `EP9_BLESS=1` only after a review) | M0 wk4 |
| Intrinsics are created in a fixed order: two isolates give the same intrinsic objects and property order | `tests::builtins::two_isolates_have_the_same_intrinsic_property_order` | M0 wk4 |
| Bytecode holds no raw pointer: operands and constants are indices or owned literals, and the VM turns them into Values per isolate at load | `roundtrip::random_units_round_trip`; `determinism::builds_on_other_threads_agree` (same bytes from other threads) | M0 wk3 |
| No heap address reaches a hash or an order, so a snapshot layout can be deterministic | `tests::strings::ep9_hash_seed_gives_the_same_hashes_in_two_runs` (`EP9_HASH_SEED`); H12 rules | M0 wk4 |
| Each native word in a heap cell has an index form that a snapshot can write. FUNCTION +24: a bytecode info = (unit, function id); a table builtin's host info = its BuiltinId (the info's place in its module's dense `infos`, `builtins::prepare`; the info does not store the id); an embedder host function (`realm::new_host_function`: unit null, function id u32::MAX) has none yet, H13-Q5. FEEDBACK info = (unit, function id) (only bytecode infos get a vector). CONTEXT scope word (OBJECT_LAYOUT 2) = (unit, scope id) | review of each `native_of` store into a cell (H25 inventory); `tests::objects::function_cell_infos_have_an_index_form` | M2-P0 |
| Units reach snapshot objects only by root or intrinsic index, or through a link cell | H35 zero-text-relocation check; SP-Unit link-cell test | SP-Unit (M0 month 2), A2 |
| Root ShapeIds are stable per engine build only; every other ShapeId is run-local (H8) | A1 snapshot test: two builds of the snapshot give the same bytes | M4 (A1) |
| One external-reference table serves the snapshot and every unit link table (03 §5) | A1 snapshot test; SP-Unit loader test | M4 (A1) |

## Consequences

- **Gains.** The snapshot, the code cache and AOT units need no object-model change. Parallel agents append ids without shifting each other's (W4-BUILTIN-IDS). Facts, hints and profiles name builtins and intrinsics by a stable index (W4-FACTS-REFS).
- **Costs.** A removed builtin or intrinsic keeps its id forever (a dead slot). Every new list needs its own golden test.
- **Follow-ups.** M1: the isolate root list, the fuse list (H31) and the external-reference table with order checks. M4: the snapshot writer and reader, and the root-shape list.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R09] | Day-1 constraints: every heap-to-native pointer goes through an external-reference table; intrinsics are created deterministically; bytecode holds constant-pool indices, never raw pointers. Pointer rewriting at deserialization works on the non-moving heap | [../research/09-critic.md](../research/09-critic.md) |
| [R14] | The dual-mode design needs a startup snapshot with stable indices: roots, builtins, ShapeIds, atoms, class root shapes | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R17] | Deterministic ShapeIds are needed only for snapshot-created root shapes; other AOT shape guards use RW cells filled at first execution | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) |

## Validation

- M0: the Rules checks above run in `cargo test` on 4 targets.
- M1 exit: every list in the table has an order check. M4 (A1): snapshot start ≥ 2x faster than without (STARTUP-SET); 100% of invalidation tests pass (07 §3).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H13-Q1 | Which lists must stay stable across engine builds, and which only within one build? A snapshot and a unit with another build id are rejected anyway (H35 S3) | M1 (index lists) | Within one build is enough for the snapshot and units. The ids that facts, hints and profiles store (builtins, intrinsics, well-known symbols, atoms) stay append-only across builds |
| H13-Q2 | `Root` has no order check: a reorder passes every test | M1 | A golden of the `Root` names, like `vm-builtin-ids.txt` |
| H13-Q3 | Is the native runtime-function table (JIT and AOT slow paths) an H13 list? | SP-Unit (M0 month 2) | Yes: units call it by index through the isolate table (04 §5 `Runtime(fn)`), so it is append-only with a golden |
| H13-Q4 | Does the global declarative record stay a plain object (`Root::Lexical`), or become a script context? | M1 (H22 realms) | Decide with H22 realms; the snapshot stores whichever exists |
| H13-Q5 | How does a snapshot write FUNCTION +24 for a host function? An embedder host function's info is a boxed FunctionInfo in `rt.infos` (unit null, function id u32::MAX, the same pair for all of them), and its `HostFn` has no index before the external-reference table exists. A table builtin's info does not store its BuiltinId | M4 (A1 snapshot writer) | Open. Proposed: an external-reference index for the embedder's `HostFn` (the one table of 03 §5), and a BuiltinId field (or module and index) in a table builtin's FunctionInfo, set by `builtins::prepare` |

## Reopens if

- The A1 snapshot gate fails (M4), or SP-Unit cannot reach an object by index.
- S3 (moving GC) starts: pointer rewriting and RW cells must then follow moves (H12).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft (rules) from 02 §6 row H13, 03 §6 and 07 §3, with the index lists as built at 55c15d0 | P0-DOCS agent (M2-P0) |
| 2026-10-22 | clarify | Index-list table re-checked at 383a317: 39 realm intrinsics (the golden lists 39), builtin blocks 1-11, and the native `RuntimeTable` is built (offset-order test, no golden). Rules re-checked: the three `native_of` stores into cells (FUNCTION info, CONTEXT scope, FEEDBACK info) each have an index form. No check changes | P2-ACCEPT-1 |
| 2026-10-22 | rewrite | Native-word Rules row: FUNCTION +24 = (unit, function id) holds for a bytecode info only. A table builtin's host info maps to its BuiltinId; an embedder host function has no index form yet (new Open question H13-Q5). This corrects the row above ("each have an index form"). New check `tests::objects::function_cell_infos_have_an_index_form` | P2 review fixer (Codex A1) |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P2-ACCEPT-1 (did not write the ADR) | Decision = the 02 §6 H13 row. Every order check in the index table exists and passes. The native-word rule holds for CONTEXT, FEEDBACK and bytecode FUNCTION infos; host FUNCTION infos do not have the (unit, function id) form (corrected after the P2 Codex review, A1: R2 below). R1: the table had the 55c15d0 counts and "not built" for the native table; re-checked at 383a317 (fixed). R2 (Codex A1, fixed by the P2 review fixer): host infos (unit null, function id u32::MAX) are not (unit, function id); Rules row qualified, H13-Q5 added. Due in M0, not filled here: H13-Q3 (P4-SP-UNIT fills it). Change-log kinds: agree | ccaf5df |
| 2 Codex | orchestrator | (filled after the P2 merge) | |
| 3 Verifier | P3-ACCEPT-2 | (filled in P3) | |
| 4 Data | from the "Validated by" line | Pass: `the_atoms_list_is_append_only`, `vm_builtin_ids_match_the_golden`, `ids_match_the_goldens`, both swap tests, `abi_version_matches_history`, `well_known_symbols_are_isolate_roots_in_order`, `atoms_and_roots_are_distinct`, both `two_isolates_have_the_same_intrinsic_property_order`, `random_units_round_trip`, `builds_on_other_threads_agree`, `ep9_hash_seed_gives_the_same_hashes_in_two_runs`. Missing: a `Root` order check (H13-Q2, M1); the SP-Unit link-cell test (P4); the A1 snapshot gates (M4) | local run, mac a64, 383a317; logs outside the repo: scratchpad m0m2/accept1/h13-ep9_vm-test.log, h13-ep9_builtins-table.log, h03-ep9_bytecode-test.log |
