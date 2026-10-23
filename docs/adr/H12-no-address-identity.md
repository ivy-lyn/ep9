# H12: No address identity

| Field | Value |
|---|---|
| Status | Draft (2026-10-20). S0 heap code (2026-10-21, `crates/ep9_heap`) has no `Drop` or finalizer hook for cells and no identity hash. M0 wk4: the `needs_drop` check is built; H12-Q1, Q2 and Q3 filled. M0 month 3 (M3-P1 B-COLLECTIONS): H12-Q1 filled again, the identity hash is built (Map, Set, WeakMap, WeakSet) |
| Fix by | M0 |
| Freeze point | M1 exit |
| Origin | v1#8; plan status U (unchanged) |
| Crates | `ep9_heap` (hash storage, finalizers), `ep9_vm` (identity hash users: Map, Set, WeakMap), `ep9_unit` and `ep9_masm` (no addresses in units) |
| Owner (plan role) | GC lead |
| Validated by | No SP-* spike. Lints (M0 wk4), unit writer checks (A2), S3 gate (M8) |
| Plan sources | [02 §6 H12](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [03 §5](../plan/03-gc.md#5-contract-items-that-exist-from-s0), [03 §1](../plan/03-gc.md#1-answer-to-hybrid-like-v8) |

## Context

- The heap is non-moving first (S0-S2). S3 adds opportunistic evacuation. Snapshots rewrite pointers at load. Units are built on one machine and loaded on another.
- Each of these breaks if an object's address leaks into a hash, a Rust structure, a finalizer or unit text.
- Julia's move to a moving GC (MMTk, ISMM'25) found address-based hashing and raw-pointer FFI among the biggest obstacles [R05].

## Decision

Identity hash stored lazily. No raw address escapes. No Rust `Drop` on heap objects. No addresses in units.

| Part | Rule | Plan source |
|---|---|---|
| Identity hash | Stored lazily on first use; never derived from the address. Header flag `hash-present` marks it (H8). Source: a per-isolate PRNG (H12-Q3) | 02 §6; 06 §2 |
| Content hashing | Tables of names hash by content, never by address: the atom table hashes the code units (SipHash-1-3, per-process key, W4-HASH), and the `Symbol.for` registry hashes the key string | M0 wk4 |
| Raw addresses | Never escape to JS, to hashes or to embedders. Off-heap pointers only in `ExternalRef<T>` (H25) | 02 §6 H12, H25 |
| `Drop` | No Rust `Drop` on heap objects. External resources use typed finalizers, never `Drop`, and count toward external-memory accounting | 03 §5 |
| Units | Unit text and rodata hold no heap address. The writer rejects HeapConst (H34, H35) | 03 §5 |

- What dual mode adds: unit text reaches heap objects only through root indices, intrinsic indices or RW cells (H13). So S3 never patches unit text; the precise visitor updates RW cells ([03 §1](../plan/03-gc.md#1-answer-to-hybrid-like-v8)).

## Rules

| Rule | Check | From |
|---|---|---|
| No `Drop` in heap types | Lint "no `Drop` in heap types" ([09 §8](../plan/09-milestones.md#8-m0-first-month-week-by-week)). Built as a build-time check: a cell kind laid out as a Rust type implements `ep9_heap::HeapLayout` and is registered with `Heap::register_typed::<T>`, which contains `const { assert!(!core::mem::needs_drop::<T>()) }`. A type with a `String` field does not build (compile_fail doctest, error E0080, `crates/ep9_heap/src/cell.rs`). The check runs where the call is instantiated: `cargo build` and `cargo test`, not `cargo check` or clippy. Kinds laid out by byte offsets (`Heap::register_kind`, every `ep9_vm` kind in wk4) have no Rust type; for them the sweep runs no Rust code | M0 wk4 |
| Names hash by content | The atom table stores each atom's content hash and its debug check (`AtomTable::check`, `crates/ep9_vm/src/atoms.rs`) rejects an entry whose stored hash is not its string's hash; the `Symbol.for` registry keys by content (P2-SYM) | M0 wk4 |
| No heap address in a hash or an order (H12-Q2) | Review rule | M0 wk4 |
| The identity hash is stored on first use and comes from the PRNG, not the address (H12-Q1) | `an_identity_hash_is_stable_hidden_and_not_an_address` (`crates/ep9_vm/src/tests/ordered_table.rs`): two isolates give the same hashes in the same order to objects at other addresses; the hash stays the same across GCs; a lookup makes none | M0 month 3 (M3-P1) |
| No HeapConst in unit text | Unit writer rejects HeapConst; xtask zero-text-relocation check | A2 |
| IR has no raw heap constants | H30 ref nodes (RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst); SP-Facts checks them | M1 |
| Identity hash survives a move | S3 evacuation tests ([03 §7](../plan/03-gc.md#7-gc-test-matrix-per-mode)) | M8 (S3) |

## Consequences

- **Gains.** Evacuation (S3), snapshots and units stay possible with no object-model change.
- **Costs.** The first identity-hash request writes the hash as one own property of the object (H12-Q1). Finalization needs a typed finalizer table instead of Rust `Drop`.
- **Follow-ups.**
  - Node-API (D4, M6) hands out references that must not be raw addresses. Objects pinned by Node-API are skipped by S3 (03 §4).
  - H25: `ExternalRef<T>` keeps a V8-style sandbox possible.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | Julia ISMM'25: address-based hashing, raw-pointer FFI and unreported roots were the biggest obstacles to a moving GC | [05-gc-design.md](../research/05-gc-design.md) |
| [R05] | Alloy (OOPSLA 2025): Rust `Drop` as GC finalizers causes soundness and premature-finalization problems; use explicit typed finalizers | [05-gc-design.md](../research/05-gc-design.md) |
| [R05] | HEADER: "Identity hash is never address-based" | [05-gc-design.md](../research/05-gc-design.md) |
| [R14] | JIT code embeds 3.0-6.5 heap objects per KB; an AOT format must turn them into root or table loads | [14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [V8-H] | V8 draws identity hashes from the isolate's PRNG and stores them in the properties field | v8 main: `src/execution/isolate.cc:7139-7146`, `src/objects/js-objects.cc:896-975` (fetched 2026-10-21) |

## Validation

- No spike in [09 §4](../plan/09-milestones.md#4-spikes) reopens H12.
- Lints at M0 wk4 (no `Vec<Local>`, no `Drop` in heap types): `cargo xtask gc-lint` (H10-Q2) and the `register_typed` compile_fail doctest in `cargo test -p ep9_heap`.
- A2: zero text relocations; HeapConst rejected; S3 evacuation of objects held by RW cells ([03 §7](../plan/03-gc.md#7-gc-test-matrix-per-mode)).
- S3 gate at M8: RSS on the JetStream 3 JS subset ≥ 15% below S2 (est.).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H12-Q1 | Where the lazy hash lives: an in-object slot, the properties field, or a side table keyed by object. The plan fixes only "stored lazily" and the `hash-present` flag | M0 wk4 (shapes v0) | **Filled (M0 month 3, M3-P1 B-COLLECTIONS; replaces W4-H12-Q1 "deferred to M1, bit reserved"): an own property under a VM-internal private name, marked by word0 flag bit 9.** The first time an object is a Map, Set, WeakMap or WeakSet key, it gets a 30-bit hash from the isolate's PRNG (`symbol::next_identity_hash`, H12-Q3). The hash is stored as an own property under a private name (a symbol with the private flag, the C8 pattern), with attributes {}, by a raw store that ignores `[[Extensible]]`, so a frozen object can be a key. `kinds::FLAG_HASH_PRESENT` (0x0200, word0 flag bit 9) marks it, so a lookup of an object without a hash reads no property and makes none. `[[OwnPropertyKeys]]` and for-in never list a private name, so JS cannot see the hash. Symbols keep their own hash (SYMBOL +16); strings hash by content (the keyed string hash); a number hashes its one canonical Value (SameValueZero: -0 is +0, one NaN). No layout change (OBJECT_LAYOUT 3). Cost: a hashed object gets one more property (a shape transition). The private name is one per isolate: a `SymbolRoots` field made by `symbol::install` (`symbol::identity_hash_key`), shared by every realm, so an object has one hash in every realm (review finding COLL-1; test `collections_hash_keys_across_realms` in `crates/ep9_builtins/tests/realm.rs`). A move to a header word is an M1 option with H17. Code: `identity_hash` in `crates/ep9_vm/src/ordered_table.rs`. Test: `an_identity_hash_is_stable_hidden_and_not_an_address` in `crates/ep9_vm/src/tests/ordered_table.rs` |
| H12-Q2 | How the "no raw address escapes" rule is checked beyond review (for example, `Debug` output of heap handles, embedder APIs) | M0 wk4 lints | **Filled (M0 wk4, W4-H12-Q2): a review rule, no lint in M0.** A reviewer rejects a heap-cell address (`CellPtr::addr`, the bits of a cell `Value`) that goes into (a) a hash input or a table key, (b) a sort or iteration order, (c) anything JS, an embedder or a unit can see. Allowed: the heap's own metadata (pages, chunks, mark bits), which S3 updates when it moves cells; `Debug` and panic text, which never reach JS; and addresses of Rust memory that is not a cell (the template cache keyed by `FunctionInfo`, `crates/ep9_vm/src/runtime.rs:269`; the code map, `crates/ep9_vm/src/dispatch.rs:180`). A review of `ep9_vm` and `ep9_heap` on 2026-10-21 found no cell address in a hash or an order |
| H12-Q3 | Hash source: a per-isolate PRNG or a counter. Must not depend on the address | M0 wk4 | **Filled (M0 wk4, W4-H12-Q3): a per-isolate PRNG.** Each isolate draws identity hashes from its own generator, seeded from the process hash key (`EP9_HASH_SEED` makes runs repeatable, W4-HASH). A counter would make hashes predictable and leak creation order; a seed shared by every run invites hash flooding (Node CVE-2017-11499, a fixed V8 snapshot seed). First user: the SYMBOL hash (P2-SYM). Precedent: V8 `Isolate::GenerateIdentityHash` draws from the isolate's `random_number_generator()` and never returns 0 (`src/execution/isolate.cc:7139-7146`) |

## Reopens if

- The S3 gate cannot move hashed objects without breaking Map/Set order or WeakMap lookup.
- A lint or fuzz finding shows an address escape that the rule cannot block.
- D6 = sandbox (11-decisions "Affects": H25, H35, H17): pointer tables and cage-relative pointers tighten this row; review it with H25.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6 and 03 §1, §5 (M0 wk1 list) | ADR agent |
| 2026-10-21 | clarify | Status: S0 heap built. A cell kind has only a trace function and a size; the sweep runs no Rust code for dead cells. No identity hash yet (H12-Q1 open) | heap agent |
| 2026-10-21 | fill | "No `Drop` in heap types" built: `HeapLayout` + `Heap::register_typed` with a const `needs_drop` assert (compile_fail doctest). Content hashing for the atom table and the `Symbol.for` registry recorded. H12-Q1 deferred to M1 with word0 bit 9 reserved; H12-Q2 review rule; H12-Q3 per-isolate PRNG. Decisions W4-LINT-NEEDS-DROP, W4-H12-Q1, W4-H12-Q2, W4-H12-Q3 | P2-LINT agent |
| 2026-10-21 | clarify | 1 `path:line` reference moved to the code at `479b1cd` (P4-DOCS drift script, [I15]) | P4-DOCS agent |
| 2026-10-21 | clarify | 2 `path:line` references moved to the code after M2-P0 (drift script, [I15]) | M2-P0 integration fixer |
| 2026-10-23 | clarify | 2 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P0 integration fixer |
| 2026-10-23 | rewrite | H12-Q1 filled again: the identity hash is an own property under a VM-internal private name with word0 flag bit 9 (`FLAG_HASH_PRESENT`), for Map, Set, WeakMap and WeakSet keys. It replaces the chosen option "deferred to M1, bit reserved" (W4-H12-Q1; plan-v8 V8-A3). A Rules row and the Costs line follow it. Kind proposed: rewrite (a chosen option is replaced) | M3-P1 B-COLLECTIONS |
| 2026-10-23 | clarify | H12-Q1: the M3-P1 interim text (the private name kept on `%Object.prototype%` under the class-fields key, one per realm) is replaced by the as-built place: one private name per isolate, a `SymbolRoots` field made by `symbol::install` (`symbol::identity_hash_key`), as the H12-Q1 text said the integration fixer would do (M3-P1 fix-realm, review finding COLL-1). The Status line names the M3-P1 fill. No rule or check changes; the new test `collections_hash_keys_across_realms` covers Map, Set, WeakMap and WeakSet across 2 realms | M3-P1 integration fixer |
| 2026-10-23 | clarify | 1 `path:line` reference moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P1 integration fixer |
| 2026-10-23 | clarify | New `## Review record` section (README §3.1; batch B5a, V8-A17) with rows 1 and 4 by M3-P2 ACCEPT-6. Rows 2 and 3 are placeholders for the orchestrator's Codex run and M3-P3 ACCEPT-7. Row 1 lists one fix (R1) and two notes for the owner; no text outside the record changes, and no check changes | M3-P2 ACCEPT-6 |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | M3-P2 ACCEPT-6 (did not write the ADR or any Change-log row before the 2026-10-23 review row) | Checked at cbae45d (the M3-P1 head). Decision = the 02 §6 H12 row, word for word ("Identity hash stored lazily. No raw address escapes. No Rust `Drop` on heap objects. No addresses in units"), plus the parts table. The H12-Q1 fill (M3-P1 B-COLLECTIONS, kind rewrite, V8-A3) against the code: `identity_hash` (ordered_table.rs:313-331 at cbae45d) reads a stored hash only when word0 flag bit 9 is set (`has_hash`, ordered_table.rs:303 at cbae45d), so a lookup with `create` false makes none; with `create` it draws a 30-bit value from the isolate's PRNG (`symbol::next_identity_hash`, a splitmix step masked to 30 bits, symbol.rs:272-287 at cbae45d, seeded from the process hash key, never from an address), stores it with `props::own_add(.., 0)` (attributes {}, a raw add that does not read [[Extensible]]) under the private name `symbol::identity_hash_key` (one `SymbolRoots` field per isolate, symbol.rs:452-454 at cbae45d) and sets `kinds::FLAG_HASH_PRESENT` = 0x0200 (kinds.rs:105 at cbae45d). Strings hash by content (`string::hash`), symbols by their own PRNG hash, other primitives by `mix` of the canonical Value bits (`hash_of`, ordered_table.rs:336-346 at cbae45d): no input is a cell address. A JS probe (`ep9_builtins`, match and sibling, with and without GC stress; node v24.22.1 prints the same lines, except that node also lists `arguments` and `caller` as own keys of a sloppy function, a V8 legacy property and not an H12 item): a frozen object is a Map, WeakSet and WeakMap key and stays frozen and not extensible; `Reflect.ownKeys`, `Object.getOwnPropertySymbols`, `Object.getOwnPropertyNames`, `JSON.stringify` and for-in show no new key; a non-extensible object, a function, an array and `globalThis` are keys and get no visible key; -0 and +0 are one key, and NaN is one key; 2,000 object keys are all found; a `get` or `has` with a new object adds no key. Rules: every check exists and passes (row 4). The H12-Q2 review rule over the M3-P1 tables: they hash no cell address (above). The other Rust maps of `ep9_vm` with a number key: the template map (`Runtime::templates`, runtime.rs:269 at cbae45d; a FunctionInfo address, Rust memory: allowed), the builtin-closure map (jobs.rs:164 at cbae45d; a Rust fn address: allowed), and N1. Status line and the README §5 cell: true. Change-log kinds: agree (the B-COLLECTIONS row replaces the chosen option "deferred to M1": rewrite, README §3.3; the fixer's per-isolate key row changes no rule or check: clarify). Open questions due in M0: none open (Q1-Q3 filled). Not fixed here (this role owns only rows 1 and 4 in H12): R1 (clarify, fixer item): the Crates line says that `ep9_heap` holds "hash storage"; as built `ep9_vm` stores the hash as an own property (`ordered_table.rs`, `props`), and `ep9_heap` holds only the word0 flag bits. N1 (older code, M0 month 2; (c) for this review): `interp/slow/eval.rs:465` (at cbae45d) builds a `HashSet<u64>` of atom Values, which are cell addresses, for a membership test inside one EvalDeclarationInstantiation step. It sets no order and nothing outside sees it, but the letter of the H12-Q2 rule (a) rejects a cell address as a table key; owner: a later VM role (a key by atom index), or an H12-Q2 note that allows it. N2: WeakMap and WeakSet values are strong (no ephemerons; M3-P1-WEAK-GAP, M1); a GC item, not an identity item | read and runs at cbae45d; probe outside the repo: scratchpad m0m3/p2/a6logs/cases.js, probe-js.log, cases.node.txt |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | (filled after the M3-P2 merge: the ACCEPT-6 docs slice) | |
| 3 Verifier | M3-P3 ACCEPT-7 | (filled in M3-P3) | |
| 4 Data | from the "Validated by" line and the H12-Q1 test (V8-A17) | The identity-hash test output at cbae45d: `cargo test -p ep9_vm --locked --lib`: 343 pass, with `tests::ordered_table` 8 pass, among them `an_identity_hash_is_stable_hidden_and_not_an_address` (no hash before the first use and none from a lookup; a 30-bit hash with flag bit 9; the same hash after a GC; no own key; a non-extensible object takes one; two isolates give the same 8 hashes to objects at other addresses, all different) and `a_dead_weak_key_is_cleared_and_its_value_freed`; `crates/ep9_builtins/tests/collections.rs`: 7 pass (`keys_are_same_value_zero_in_insertion_order`, `weak_collections_take_objects_and_unregistered_symbols`, `own_properties_match_node` among them); `crates/ep9_builtins/tests/realm.rs`: 7 pass, with `collections_hash_keys_across_realms` (one hash per object in 2 realms). "No `Drop` in heap types": `cargo test -p ep9_value -p ep9_heap --locked` passes, with the `register_typed` compile_fail doctest. Missing: the unit writer checks (A2) and the S3 gate (M8), as "Validated by" says | local run, mac a64, cbae45d, `EP9_HASH_SEED=1`; logs outside the repo: scratchpad m0m3/p2/a6logs/vm-lib.log, builtins.log, value-heap.log |
