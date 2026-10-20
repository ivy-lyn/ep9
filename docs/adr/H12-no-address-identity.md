# H12: No address identity

| Field | Value |
|---|---|
| Status | Draft (2026-10-20) |
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
| Identity hash | Stored lazily on first use; never derived from the address. Header flag `hash-present` marks it (H8) | 02 §6; 06 §2 |
| Raw addresses | Never escape to JS, to hashes or to embedders. Off-heap pointers only in `ExternalRef<T>` (H25) | 02 §6 H12, H25 |
| `Drop` | No Rust `Drop` on heap objects. External resources use typed finalizers, never `Drop`, and count toward external-memory accounting | 03 §5 |
| Units | Unit text and rodata hold no heap address. The writer rejects HeapConst (H34, H35) | 03 §5 |

- What dual mode adds: unit text reaches heap objects only through root indices, intrinsic indices or RW cells (H13). So S3 never patches unit text; the precise visitor updates RW cells ([03 §1](../plan/03-gc.md#1-answer-to-hybrid-like-v8)).

## Rules

| Rule | Check | From |
|---|---|---|
| No `Drop` in heap types | Lint "no `Drop` in heap types" ([09 §8](../plan/09-milestones.md#8-m0-first-month-week-by-week)) | M0 wk4 |
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

## Validation

- No spike in [09 §4](../plan/09-milestones.md#4-spikes) reopens H12.
- Lints at M0 wk4 (no `Vec<Local>`, no `Drop` in heap types).
- A2: zero text relocations; HeapConst rejected; S3 evacuation of objects held by RW cells ([03 §7](../plan/03-gc.md#7-gc-test-matrix-per-mode)).
- S3 gate at M8: RSS on the JetStream 3 JS subset ≥ 15% below S2 (est.).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H12-Q1 | Where the lazy hash lives: an in-object slot, the properties field, or a side table keyed by object. The plan fixes only "stored lazily" and the `hash-present` flag | M0 wk4 (shapes v0) | Decide with H17 shapes; record as a fill |
| H12-Q2 | How the "no raw address escapes" rule is checked beyond review (for example, `Debug` output of heap handles, embedder APIs) | M0 wk4 lints | Review rule; no lint in M0 |
| H12-Q3 | Hash source: a per-isolate PRNG or a counter. Must not depend on the address | M0 wk4 | Per-isolate PRNG |

## Reopens if

- The S3 gate cannot move hashed objects without breaking Map/Set order or WeakMap lookup.
- A lint or fuzz finding shows an address escape that the rule cannot block.
- D6 = sandbox (11-decisions "Affects": H25, H35, H17): pointer tables and cage-relative pointers tighten this row; review it with H25.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6 and 03 §1, §5 (M0 wk1 list) | ADR agent |
