# H08: Object header and ShapeId

| Field | Value |
|---|---|
| Status | Draft (2026-10-20; Q1 and Q3 filled 2026-10-21: shape region, word0 flag bits; Q2 filled 2026-10-22: no shape-guarded load in Spike B). Accept needs the review record (README §3.1) and SP-Unit |
| Fix by | M0 wk2 |
| Freeze point | M1 exit |
| Origin | v1#4 + R16-D5 (as corrected by R17); plan status E (extended: ShapeId rule) |
| Crates | `ep9_heap` (word0, gc_state, kind, shape region), `ep9_vm` (shapes, ICs), `ep9_unit` (layout version), `ep9_opt` / `ep9_baseline` (AOT shape guards) |
| Owner (plan role) | VM lead + GC lead; AOT senior for the ShapeId rule (M0 wk2) |
| Validated by | SP-Value (word0 = 8 B); SP-Unit (first AOT code) |
| Plan sources | [02 §6 H8](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [06 §2](../plan/06-object-model-and-spec.md#2-header-and-objects-h8-extended), [06 §2.1](../plan/06-object-model-and-spec.md#21-shapeid-rule-h8), [03 §5](../plan/03-gc.md#5-contract-items-that-exist-from-s0) |

## Context

- The header is read by every IC guard, the GC visitor, the barrier and (later) AOT shape guards.
- AOT text is shared per process and may be signed. It cannot hold run-local shape addresses or ids (H12, H35).
- R16-D5 asked for deterministic transition trees so build-time ShapeIds match at run time. R17 showed that RW cells remove that need.
- Goals: G2-G4 (one-compare IC guards), G11 (dual-mode contracts from M0).

## Decision

8-byte word0: u32 ShapeId, u8 gc_state, u8 kind, u16 flags; size from header; minimum object 16 B. ShapeId = offset into the shape space, O(1) to Shape. Snapshot root shapes have stable ShapeIds; other ShapeIds are run-local and never appear in AOT text. AOT guards compare RW cells filled at first execution.

```
word0 (8 bytes, Relaxed atomic):
  bytes 0-3  shape_id  u32   ShapeId: offset into the shape space, O(1) to the Shape; IC guards compare it
  byte  4    gc_state  u8    cell state for the barrier: BLACK=1, WHITE=2, GREY=3 (H11); the mark bit is in side metadata (H9)
  byte  5    kind      u8    GC visitor and size without loading the shape
  bytes 6-7  flags     u16   hash-present, pinned, is-prototype, ...
```

| ShapeId kind | Created by | Stable across runs? | In AOT text? | AOT shape guard |
|---|---|---|---|---|
| Snapshot root shape | The startup snapshot, with a deterministic layout (H13) | Yes, per engine build | Yes, as an immediate (ShapeRef) | `header.shape_id == imm` |
| Every other shape | Transitions at run time | No (run-local) | Never | `header.shape_id == RW cell`; the first execution fills the cell |

```
 AOT text (RX, one copy per process)            unit RW data (one copy per isolate, H22)
 ld   s, [obj + 0]       ; header ShapeId       IcCell[k] = { shape_id, slot }
 ld   c, [rw + k]        ; pc-relative cell       empty at load  -> the first run misses
 cmp  s, c.shape_id                               the IC slow path fills the cell
 b.ne miss               ; H32 policy path        later runs hit; a dead shape clears it (weak)
 ld   v, [obj + c.slot]
```

- The object-layout version is a unit header field (H35). A change rejects the unit's native code; its bytecode still runs.
- word0 can hold a forwarding pointer in S3; the state stays in gc_state ([R05] HEADER).

### Shape space (H08-Q1, filled 2026-10-21)

```
cage base (4 GiB-aligned outside Miri)
[ shape region: chunks [0, N), N * 256 KiB = shape_space_bytes ][ ordinary pages, LOS ...... ]
  ^ offset 0 = page header, never a cell             (alloc, LOS, the page cache never use [0, N))

ShapeId(shape) = (shape address - cage base) as u32      Heap::shape_id_of
shape(id)      = cage base + id                          Heap::shape_cell; JIT/AOT: [heap + 24] + id
```

| Item | Value | Where |
|---|---|---|
| Location | The first `HeapConfig::shape_space_bytes` of the cage; the cage base is at `Heap::CAGE_BASE_OFFSET` = 24 (offsets 0, 8, 16 unchanged) | `crates/ep9_heap/src/memory.rs`, `heap.rs` |
| Size | 0 = no shape space (the `ep9_heap` default, so heap tests and Miri are unchanged); otherwise a power of two from 1 MiB to 4 GiB, at most the cage size. `ep9_vm` asks for 256 MiB through `shape::heap_config` (M0 wk4) | `HeapConfig::shape_space_bytes`, `check_shape_space` |
| Alignment | Outside Miri a shape space needs `cage.align >= 4 GiB` (the default), so the id is also the low 32 bits of the address (JSC StructureID) and an IC compares `(key as u32) == word0.shape_id`. Under Miri this holds for `ep9_heap` tests only: their stand-in reservation (at most 8 MiB) is aligned to 256 KiB, so they use the subtract (`Heap::shape_id_of`). No `ep9_vm` isolate starts under Miri: its 256 MiB shape space does not fit the stand-in ("shape_space_bytes is larger than the heap reservation"), and `shape::init` asserts a `CAGE_ALIGN` cage (M2-P1-MIRI-VM; H08-Q4) | `HeapError::Config` otherwise |
| Id 0 | Offset 0 holds a page header, so no shape has id 0: 0 means "none". Bits 0-2 are always 0 (8 B granule) | static assert + `alloc_shape` assert |
| Cells | Only `Space::Shape` kinds (`Heap::register_kind_in`), only through `Heap::alloc_shape` (at most 8 KiB). `Heap::alloc` is unchanged and does not list them (`UnknownKind`); `alloc_shape` refuses ordinary kinds (`WrongSpace`) | `crates/ep9_heap/src/heap.rs` |
| GC | Shape pages are small pages: marked, found by the conservative scan, swept lazily (own queue), and an empty one goes back to the region decommitted. A full region gives `OutOfMemory` after one GC | `crates/ep9_heap/src/tests/shape.rs` |

- An IC key, and the `shape_id` of an AOT RW `IcCell` in the sketch above, is a weak shape Value whose low 32 bits are the ShapeId. A GC clears a dead key to `undefined` (0x0a); that low half is never a ShapeId (bits 0-2 are 0), so a reused address never gives a false hit.

### Flag bits (H08-Q3, filled 2026-10-21)

| Bits | Meaning | Where |
|---|---|---|
| 0-7 | Kind-specific. JS objects: the in-object slot count (0-255, H17). Strings: bit 0 = atom (H19) | `kinds::FLAG_INOBJECT_MASK`, `kinds::FLAG_ATOM` |
| 8 | is-prototype: the object is some object's `[[Prototype]]` and has a unique shape (H17 prototype mode) | `kinds::FLAG_IS_PROTOTYPE` |
| 9 | Reserved for hash-present (identity hash, not built) | `kinds::FLAG_HASH_PRESENT` |
| 10-15 | Reserved (pinned, if S3 needs it, takes one of these) | none |

- `ep9_vm` asks for a cage aligned to 64 GiB and at most 64 GiB in size (`shape::heap_config`, `CAGE_ALIGN`). The H08-Q1 rule (align >= 4 GiB) still holds. With it, `shape_of(o) = (o & !(64 GiB - 1)) | shape_id` needs no heap pointer (decision W4-OBJ-CAGE).

## Rules

| Rule | Check | From |
|---|---|---|
| word0 is 8 B; minimum object is 16 B | Static asserts in `ep9_value` (`size_of::<Word0>() == 8` and the word0 field offsets; SP-Value); `ep9_heap` `MIN_CELL_SIZE` = 16, and every cell size rounds up to it. No xtask task checks sizes yet (08 §7 lists size asserts) | M0 wk2 |
| Object size is computable from the header (plus a length field) | Heap verifier; heap stays parsable (H9) | S0 |
| No run-local ShapeId in AOT text or rodata | IR has no raw ShapeId for non-root shapes (ShapeRef lowers to CellRef, H30); unit golden tests | A2 |
| Only snapshot root shapes have stable ShapeIds | Deterministic snapshot layout (H13) | A1 (M4) |
| Unit IC cells hold shapes weakly | GC rule for unit RW data (03 §2) | A2 |
| The shape region never moves; S3 evacuation excludes it | Non-moving S0; a shape page is only ever freed whole (`free_small_page`) | M0 wk4 |
| A bare u32 ShapeId appears only in word0, in caches that every GC flushes, and in root-shape immediates | Code review (H16 IC format); the heap verifier checks every word0 id | M0 wk4 |
| IC and feedback data hold shapes as weak Values (the shape cell pointer), never as bare ids | H16 entry `[key (weak) \| value (strong)]`; `Tracer::visit_weak` | M0 wk4 |
| The heap marks the shape that word0 names; no trace function visits it | `Tracer::drain`; test `a_shape_referenced_only_from_word0_survives` | M0 wk4 |
| With a shape space, every non-zero word0 ShapeId names a live shape-space cell | Heap verifier (`verify_word0_shape`), run after every GC under GC-stress | M0 wk4 |
| The shape region and ordinary pages share `max_heap_bytes`. An allocation that finds no memory after its GC sweeps both lazy sweep queues and retries once, with no second GC, before it gives `OutOfMemory` | `Inner::sweep_before_out_of_memory`; tests `alloc_shape_sweeps_dead_ordinary_pages_before_out_of_memory`, `alloc_sweeps_dead_shape_pages_before_out_of_memory`, `a_large_cell_sweeps_dead_shape_pages_before_out_of_memory` (verifier off: it finishes the sweep and hides the case) | M0 wk4 |

## Consequences

- **Gains.** IC guard = one u32 compare. GC needs no shape load; the barrier reads one byte.
  - RW cells remove the need for deterministic transition trees. Property-add order, slack tracking and dictionary heuristics need not match between build and run [R17].
- **Costs.** AOT guards on non-root shapes cost one extra pc-relative load. The first execution of each AOT site misses.
- **Follow-ups.** H13 must make the snapshot layout deterministic. H30 must lower ShapeRef to an immediate only for snapshot root shapes. H35 carries the object-layout version.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | HEADER: u32 shape, u8 gc_state, u8 kind, u16 flags; size from header; minimum 16 B; forwarding in word0 | [05-gc-design.md](../research/05-gc-design.md) |
| [R14] | Static Hermes AOT keeps per-site `{clazz, slot}` RW entries: 282 read and 128 write entries for Richards | [14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R17] | Correction of R16-D5: deterministic ShapeIds only for snapshot root shapes; other guards use RW cells | [17-aot-refutation.md](../research/17-aot-refutation.md) |
| [R16] | The corrected proposal R16-D5 | [16-aot-synthesis.md](../research/16-aot-synthesis.md) |

## Validation

- **SP-Value** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk2): word0 = 8 B. M0 exit item 8.
- **SP-Unit** (M0 month 2, inside Spike B): first AOT code; its 10 opcodes load, call, unwind, GC-scan and fall back (reopens H34, H35).
- A2 gate: GC-stress clean with AOT frames; 100% of header mutations end in their [H35](H35-aot-unit-format.md) S3 outcome.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H08-Q1 | Size, alignment and location of the shape space. A u32 offset bounds it; the plan gives no number | M0 wk4 (shapes v0) | **Filled 2026-10-21:** the first `shape_space_bytes` of the cage (0 = none in `ep9_heap`; 256 MiB from `ep9_vm`); a power of two from 1 MiB to 4 GiB; cage align >= 4 GiB outside Miri; ShapeId = offset from the cage base. See "Shape space" under Decision |
| H08-Q2 | Do SP-Unit's 10 opcodes include a shape-guarded load, so the RW-cell guard is tested in M0? | Spike B plan | **Filled 2026-10-22 (M2-H08-Q2): no, not in M0.** The 11 Spike B opcodes (09 §4 says 10; Spike B adds AddSmi) have no property load, and the spike JIT records no feedback. The RW-cell shape guard is tested in M1 (Spike C). The old default was "include one property load" |
| H08-Q3 | Full flag-bit list beyond hash-present, pinned, is-prototype | M1a (H17 freeze) | **Filled 2026-10-21:** bits 0-7 kind-specific, bit 8 is-prototype, bit 9 reserved for hash-present, bits 10-15 reserved. See "Flag bits" under Decision |
| H08-Q4 | Miri for `ep9_vm`: a stand-in cage with `CAGE_ALIGN` alignment, or the subtract path in `shape_of` under Miri (M2-P1-MIRI-VM)? | M1 | Heap owner (GC lead) decides |

## Reopens if

- SP-Value fails the word0 = 8 B assert (09 §4 names H6 and H11; word0 is shared with H6's gate).
- H13 cannot give a deterministic snapshot layout. Then root-shape immediates have no basis.
- A spike shows that the RW-cell guard fails a correctness check (SP-Unit reopens H34 and H35; this row is reviewed with them).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6, 06 §2 and §2.1 (M0 wk1 list) | ADR agent |
| 2026-10-20 | clarify | word0 byte 4 comment: gc_state values BLACK=1, WHITE=2, GREY=3 from the H11 decision; the mark bit is in side metadata (H9). Layout unchanged | apply agent |
| 2026-10-21 | clarify | Validation: A2 gate wording "header-mutation fallbacks 100%" now uses the H35-fallback text ("100% of header mutations end in their H35 S3 outcome"), as in 09 §3 and 05 §12. No check changes | hygiene agent |
| 2026-10-21 | fill | H08-Q1 filled: shape region = the first `shape_space_bytes` of the cage (power of two, 1 MiB-4 GiB, 0 = none); cage align >= 4 GiB outside Miri; ShapeId = (addr - cage base) as u32; id 0 = none; cage base at heap offset 24. Rules added: the region never moves; where a bare ShapeId may appear; IC data holds shapes weakly; the heap marks word0's shape; the verifier checks word0 ids. Decisions W4-SHAPEID and W4-SHAPE-ALLOC (decisions log §11) | P1-HEAP agent |
| 2026-10-21 | fill | Rule added: both spaces share `max_heap_bytes`, so before `OutOfMemory` an allocation sweeps both lazy sweep queues and retries once (no second GC). A page that died in the allocation's own GC used to wait in the other space's queue (Codex review of P1) | P1 fix agent |
| 2026-10-21 | fill | H08-Q3 filled: bits 0-7 kind-specific (in-object slot count for JS objects, atom for strings), bit 8 is-prototype, bit 9 reserved for hash-present, bits 10-15 reserved. Note: `ep9_vm` aligns its cage to 64 GiB so an object's shape needs no heap pointer (W4-OBJ-CAGE) | P2-OBJ agent |
| 2026-10-22 | clarify | Status line names the Q1 and Q3 fills and what Accept needs (README §5 already lists the fills). No check changes | P2-ACCEPT-1 |
| 2026-10-22 | clarify | Shape space, Alignment row: the Miri text holds for `ep9_heap` tests only; no `ep9_vm` isolate starts under Miri (M2-P1-MIRI-VM). New Open question H08-Q4. "(P2)" in the Size row now says "(M0 wk4)". No check changes | P2-ACCEPT-1 |
| 2026-10-22 | clarify | Rules row 1: the size checks are `const` asserts in `ep9_value` and `MIN_CELL_SIZE` in `ep9_heap`; no xtask task checks sizes. No check changes | P2-ACCEPT-1 |
| 2026-10-22 | fill | H08-Q2 filled with the orchestrator decision M2-H08-Q2: the RW-cell shape guard is not tested in M0; Spike C tests it in M1. Every existing check keeps its result | P2-ACCEPT-1 (decision: orchestrator) |
| 2026-10-22 | clarify | Status line also names the H08-Q2 fill (2026-10-22), as the README §5 index row does. No check changes | P3-ACCEPT-2 |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P2-ACCEPT-1 (did not write the ADR) | Decision = the 02 §6 H8 row, word for word. Rules checked against `ep9_value` (`Word0`, const asserts), `ep9_heap` (shape space, verifier, the sweep-before-OOM tests) and `ep9_vm` (`shape::heap_config`, `CAGE_ALIGN`, the flag constants). R1: the Miri text holds for `ep9_heap` only; new Q4 (fixed). R2: there is no xtask size task; the check names the real asserts (fixed). R3: Status line (fixed). R4: Q2 filled (M2-H08-Q2). R5: "(P2)" was unclear (fixed). Change-log kinds: agree | ccaf5df |
| 2 Codex | orchestrator | (filled after the P2 merge) | |
| 3 Verifier | P3-ACCEPT-2 | (filled in P3) | |
| 4 Data | from the "Validated by" line | SP-Value: the `ep9_value` const asserts compile and `cargo test -p ep9_value --locked` passes; SP-Value GO (decisions log W4-P4-SPIKE-RECHECK). `cargo test -p ep9_heap --locked` passes, with `a_shape_referenced_only_from_word0_survives` and the 3 sweep-before-OOM tests. Missing: SP-Unit (P4-SP-UNIT); the A2 gate (A2) | local run, mac a64, 383a317; logs outside the repo: scratchpad m0m2/accept1/h08-ep9_value-test.log, h08-ep9_heap-test.log |
