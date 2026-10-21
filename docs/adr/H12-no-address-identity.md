# H12: No address identity

| Field | Value |
|---|---|
| Status | Draft (2026-10-20). S0 heap code (2026-10-21, `crates/ep9_heap`) has no `Drop` or finalizer hook for cells and no identity hash. M0 wk4: the `needs_drop` check is built; H12-Q1, Q2 and Q3 filled |
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
| No HeapConst in unit text | Unit writer rejects HeapConst; xtask zero-text-relocation check | A2 |
| IR has no raw heap constants | H30 ref nodes (RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst); SP-Facts checks them | M1 |
| Identity hash survives a move | S3 evacuation tests ([03 §7](../plan/03-gc.md#7-gc-test-matrix-per-mode)) | M8 (S3) |

## Consequences

- **Gains.** Evacuation (S3), snapshots and units stay possible with no object-model change.
- **Costs.** The first identity-hash request writes the hash somewhere (Open question H12-Q1). Finalization needs a typed finalizer table instead of Rust `Drop`.
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
| H12-Q1 | Where the lazy hash lives: an in-object slot, the properties field, or a side table keyed by object. The plan fixes only "stored lazily" and the `hash-present` flag | M0 wk4 (shapes v0) | **Filled (M0 wk4, W4-H12-Q1): deferred to M1, bit reserved.** No object needs an identity hash in week 4 (no Map, Set or WeakMap yet), so where it lives is chosen with them in M1, against the H17 shapes then built. word0 flag bit 9 is reserved for `hash-present` (`kinds::FLAG_HASH_PRESENT` = 0x0200, `crates/ep9_vm/src/kinds.rs:89-90`; W4-WORD0-FLAGS). Symbols do not wait: the SYMBOL cell holds its hash at +16 (P2-SYM). Precedent: V8 keeps an object's hash in its properties field (a Smi when there are no out-of-object properties, else in the PropertyArray or dictionary; `JSReceiver::SetIdentityHash`, `src/objects/js-objects.cc:896-975`) |
| H12-Q2 | How the "no raw address escapes" rule is checked beyond review (for example, `Debug` output of heap handles, embedder APIs) | M0 wk4 lints | **Filled (M0 wk4, W4-H12-Q2): a review rule, no lint in M0.** A reviewer rejects a heap-cell address (`CellPtr::addr`, the bits of a cell `Value`) that goes into (a) a hash input or a table key, (b) a sort or iteration order, (c) anything JS, an embedder or a unit can see. Allowed: the heap's own metadata (pages, chunks, mark bits), which S3 updates when it moves cells; `Debug` and panic text, which never reach JS; and addresses of Rust memory that is not a cell (the template cache keyed by `FunctionInfo`, `crates/ep9_vm/src/runtime.rs:237`; the code map, `crates/ep9_vm/src/dispatch.rs:180`). A review of `ep9_vm` and `ep9_heap` on 2026-10-21 found no cell address in a hash or an order |
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
