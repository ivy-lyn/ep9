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
 ld   c, [x27 + k]       ; unit-data register     empty at load  -> the first run misses
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
| 9 | Hash-present: the object has an identity hash (built in M3-P1 B-COLLECTIONS; set by `ordered_table::identity_hash`, H12-Q1) | `kinds::FLAG_HASH_PRESENT` |
| 10-15 | Reserved (pinned, if S3 needs it, takes one of these) | none |

- `ep9_vm` asks for a cage aligned to 64 GiB and at most 64 GiB in size (`shape::heap_config`, `CAGE_ALIGN`). The H08-Q1 rule (align >= 4 GiB) still holds. With it, `shape_of(o) = (o & !(64 GiB - 1)) | shape_id` needs no heap pointer (decision W4-OBJ-CAGE).

## Rules

| Rule | Check | From |
|---|---|---|
| word0 is 8 B; minimum object is 16 B | Static asserts in `ep9_value` (`size_of::<Word0>() == 8` and the word0 field offsets; SP-Value); `ep9_heap` `MIN_CELL_SIZE` = 16, and every cell size rounds up to it. `cargo xtask m0-exit` item 8 checks that the `Word0` size assert is there and runs `cargo check -p ep9_value --lib --target <host>`, which evaluates the `ep9_value` asserts (NOT-RUN with `--no-build-check`; `tools/xtask/src/m0_exit.rs:1270-1306`); no xtask task checks `MIN_CELL_SIZE` (08 §7 lists size asserts) | M0 wk2 |
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
- **Costs.** AOT guards on non-root shapes cost one extra load through the unit-data register (H35-Q9). The first execution of each AOT site misses.
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
- **SP-Unit** (M3-P1, moved from M0 month 2 by M2-D21-FAIL): first AOT code; its 11 opcodes (the Spike B set with AddSmi) load, call, unwind, GC-scan and fall back (reopens H34, H35).
- A2 gate: GC-stress clean with AOT frames; 100% of header mutations end in their [H35](H35-aot-unit-format.md) S3 outcome.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H08-Q1 | Size, alignment and location of the shape space. A u32 offset bounds it; the plan gives no number | M0 wk4 (shapes v0) | **Filled 2026-10-21:** the first `shape_space_bytes` of the cage (0 = none in `ep9_heap`; 256 MiB from `ep9_vm`); a power of two from 1 MiB to 4 GiB; cage align >= 4 GiB outside Miri; ShapeId = offset from the cage base. See "Shape space" under Decision |
| H08-Q2 | Do SP-Unit's 11 opcodes include a shape-guarded load, so the RW-cell guard is tested in M0? | Spike B plan | **Filled 2026-10-22 (M2-H08-Q2): no, not in M0.** The 11 Spike B opcodes (09 §4 said 10 until 2026-10-22; Spike B adds AddSmi) have no property load, and the spike JIT records no feedback. The RW-cell shape guard is tested in M1 (Spike C). The old default was "include one property load" |
| H08-Q3 | Full flag-bit list beyond hash-present, pinned, is-prototype | M1a (H17 freeze) | **Filled 2026-10-21:** bits 0-7 kind-specific, bit 8 is-prototype, bit 9 reserved for hash-present (built since M3-P1, H12-Q1), bits 10-15 reserved. See "Flag bits" under Decision |
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
| 2026-10-22 | clarify | Rules row 1: m0-exit item 8 also compiles the `ep9_value` asserts (`cargo check -p ep9_value` for the host target), so "no xtask task checks sizes" (here and in the 2026-10-22 P2-ACCEPT-1 row) was false; the cell now says what item 8 runs, as H06 Rules row 1 does (Codex C3 on H06, M2-P4-B2-ROW2). No check changes | P4 integration fixer |
| 2026-10-22 | clarify | SP-Unit has 11 opcodes (09 §4 now says 11; Spike B adds AddSmi) and runs in M3-P1 (M2-D21-FAIL): the Validation line and the H08-Q2 question. The H08-Q2 answer does not change. No check changes | P5-DOCS-SYNC agent (M2-P5) |
| 2026-10-23 | clarify | 1 `path:line` reference moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P0 integration fixer |
| 2026-10-23 | rewrite | The AOT guard picture and Costs say the RW cell is reached through the unit-data register x27 / r14, not pc-relative (H35-Q9 filled by M3-P1 SP-UNIT, which names this a rewrite: one per-process text cannot reach a per-isolate RW copy pc-relative). The guard and its cost (one extra load) do not change; no check changes | M3-P1 SP-UNIT agent |
| 2026-10-23 | clarify | Flag bit 9 hash-present is built (M3-P1 B-COLLECTIONS, H12-Q1): `ordered_table::identity_hash` sets `kinds::FLAG_HASH_PRESENT` when an object gets its identity hash. The Flag bits row and H08-Q3 said "reserved, not built". The bit value does not change. No rule or check changes | M3-P1 integration fixer |
| 2026-10-23 | clarify | Review record row 3: the `path:line` refs keep the lines of the commit the row names (e719ffa), M3-P1-REVIEWROW-REFS (X-FD1): shape.rs 369 -> 361 (1709c9e and earlier ADRLINE runs had moved them to later trees). Text only; no check changes | M3-P1 docs fixer |
| 2026-10-23 | clarify | Review record: a row-1 addendum (V8-A25) by M3-P1 ACCEPT-5 step 2 over the M3-P1 SP-UNIT lines (the AOT guard picture, Costs and their 2026-10-23 rewrite row): the text matches H35-Q9 and the unit code; kind rewrite confirmed. No text outside the record changes, and no check changes | M3-P1 ACCEPT-5 (step 2) |
| 2026-10-23 | clarify | Review record row 3 (M3-P2 ACCEPT-6): the verify of the rows written after the P3 verify and of SP-Unit, at cbae45d. No text outside the record changes; no check changes | M3-P2 ACCEPT-6 |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P2-ACCEPT-1 (did not write the ADR) | Decision = the 02 §6 H8 row, word for word. Rules checked against `ep9_value` (`Word0`, const asserts), `ep9_heap` (shape space, verifier, the sweep-before-OOM tests) and `ep9_vm` (`shape::heap_config`, `CAGE_ALIGN`, the flag constants). R1: the Miri text holds for `ep9_heap` only; new Q4 (fixed). R2: there is no xtask size task; the check names the real asserts (fixed). R3: Status line (fixed). R4: Q2 filled (M2-H08-Q2). R5: "(P2)" was unclear (fixed). Change-log kinds: agree | ccaf5df |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | No finding on H08. Codex ran on the "ADR docs" slice (31 KB) of the P2 diff at e293355 (M2-P2-REVIEW). Verdict needs-attention with 2 findings, A1 (H13) and A2 (H27, H36); a verifier reproduced both: real, category (b), fixed in P2 | decisions-log M2-P2-REVIEW, M2-P2-A1, M2-P2-A2; Codex text outside the repo: scratchpad m0m2/p2-codex-bcrhkdqte.md |
| 3 Verifier | P3-ACCEPT-2 (wrote none of the rows it checks) | Checked at e719ffa. No Codex finding to sort. P2-ACCEPT-1 rows: the Alignment row (the Miri stand-in is at most 32 pages = 8 MiB, aligned to 256 KiB, `crates/ep9_heap/src/memory.rs:182`; `shape::init` asserts a `CAGE_ALIGN` cage, `crates/ep9_vm/src/shape.rs:361`: true); Rules row 1 (the const asserts in `crates/ep9_value/src/lib.rs:44-55`, `MIN_CELL_SIZE` = 16 in `crates/ep9_heap/src/layout.rs:36`, no xtask size task: true); the H08-Q2 fill (decision M2-H08-Q2; the 11 Spike B opcodes, `spikes/spike-b/src/jit/mod.rs:47-57`, have no property load: true); the Status row. V1: the Status line did not name the Q2 fill that README §5 names (fixed, clarify). Part 4 again at e719ffa: `cargo test -p ep9_value --locked` 20 pass; `cargo test -p ep9_heap --locked` 84 pass, with `a_shape_referenced_only_from_word0_survives` and the 3 sweep-before-OOM tests. Still missing: SP-Unit (P4); H08 is verified again by P5-ACCEPT-4 after SP-Unit. Change-log kinds: agree | c68971a; local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/value.log, heap.log |
| 4 Data | from the "Validated by" line | SP-Value: the `ep9_value` const asserts compile and `cargo test -p ep9_value --locked` passes; SP-Value GO (decisions log W4-P4-SPIKE-RECHECK). `cargo test -p ep9_heap --locked` passes, with `a_shape_referenced_only_from_word0_survives` and the 3 sweep-before-OOM tests. Missing: SP-Unit (P4-SP-UNIT); the A2 gate (A2) | local run, mac a64, 383a317; logs outside the repo: scratchpad m0m2/accept1/h08-ep9_value-test.log, h08-ep9_heap-test.log |
| 1 Reviewer (M3-P1 addendum, V8-A25: SP-UNIT's AOT guard picture and Costs lines and their 2026-10-23 rewrite row) | M3-P1 ACCEPT-5 step 2 (did not write these lines) | Checked at 80b7986. The guard picture (Decision: `ld c, [x27 + k]`, unit-data register) and Costs ("one extra load through the unit-data register (H35-Q9)") match H35 (Decision, S5, H35-Q9) and 06 §2.1, which 9acc400 changed in the same way. They match the unit code: a call site reads and fills its IC cell as `[x27 + ic]` / `[r14 + ic]` (a64.rs:296-299, x64.rs:275-277 at 80b7986), with x27 / r14 = `cx.unit_data[slot]`; check (c) of `gate.sh unit` reads the RW words through the unit text and shows that the IC cell is weak (a GC clears it). The shape guard itself is not built yet: the 11 SP-Unit opcodes have no property load (H08-Q2), so Spike C (M1) tests the guard. "One extra load" holds: the cell load is one load at a fixed offset from x27, and the two x27 entry loads are paid once per function (H35 Costs), not per guard. Kind: rewrite, agree. The line is in the Decision section, and the way the RW cell is reached (a register, not pc-relative) is a choice that code depends on (README §3.3). It is pre-freeze churn (H08 freezes at M1 exit), so G8 does not count it. Not covered here: the other M3-P1 rows (the fixer's flag bit 9 clarify, ADRLINE). Row 4 still says "Missing: SP-Unit"; SP-Unit now has data (H35 row 4), for ACCEPT-6 row 3 | read at 80b7986; gate output outside the repo: scratchpad m0m3/p1/a5s2logs/gate-unit.log |
| 3 Verifier (M3-P2: the rows written after the P3 verify, and SP-Unit) | M3-P2 ACCEPT-6 (wrote none of the rows it checks and no H08 text) | Checked at cbae45d (spikes/ has no change since 80b7986). Edits by others after the P3-ACCEPT-2 row 3 (e719ffa), each checked: 7b99a01 (Rules row 1: m0-exit item 8 checks the `Word0` assert and runs `cargo check -p ep9_value`, m0_exit.rs:1270-1306 at cbae45d: true; clarify, agree); b027ceb (SP-Unit in M3-P1 with 11 opcodes; H08-Q2 text: true; clarify, agree); d6c8620 and 1709c9e (ADRLINE; the dated row-3 ref was put back to e719ffa by 25fd20a: `shape.rs:361` at e719ffa is the `assert!` of `shape::init`: true); 9acc400 (the AOT guard picture and Costs through the unit-data register; kind rewrite, pre-freeze churn: agree); 65fed38 (flag bit 9 is built: `kinds::FLAG_HASH_PRESENT` = 0x0200, set by `ordered_table::identity_hash`, ordered_table.rs:329 at cbae45d: true; clarify, agree, the bit value does not change); b92754d (the ACCEPT-5 step 2 addendum: its refs at 80b7986 name the same code at cbae45d, a64.rs:296-299 `ldr x10, [x27, ic]` / `str x9, [x27, ic]`). Codex runs over these edits: the P5 docs run (base bdd2009) had D1 (the SP-Unit schedule; the b027ceb H08 lines were already right); the M3-P1 fixer docs slice (base 94ed243, head b92754d, scratchpad m0m3/p1/codex-fix-docs.out; it holds 65fed38, 1709c9e and b92754d): no finding on H08, and its text says the H08 addendum matches the unit-register code; X-FD1 touched this ADR's P3 row 3 (fixed, 25fd20a). The SP-UNIT commit 9acc400 was in no Codex diff (the SP-UNIT Codex slice stopped at the content filter), so the M3-P1 Claude reviewer covered SP-UNIT and the addendum row 1 reviewed its H08 lines. The orchestrator fills a row-2 addendum from these runs. No open Codex finding on H08. SP-Unit, the data that rows 3 and 4 waited for: `bash spikes/spike-b/gate.sh unit` PASS on mac a64 at cbae45d (11 opcodes: load, call, unwind, GC-scan, fall back; check (c): the IC cell is reached as [x27 + ic] and is weak, a GC clears it). The shape guard itself is not in SP-Unit (H08-Q2: no property load in the 11 opcodes); Spike C (M1) tests it, as H08-Q2 says. Other targets (the orchestrator's runs, the same code tree): x86 sandbox `spike-b-unit-gate` PASS, Docker linux a64 PASS (local CI at e86ecae); windows-x64 CI probe 39886596197 PASS. Data at cbae45d: `cargo test -p ep9_value -p ep9_heap --locked`: 107 pass (with `a_shape_referenced_only_from_word0_survives` and the sweep-before-out-of-memory tests); `cargo test -p ep9_vm --locked --lib`: 343 pass. Open questions due in M0: none (Q1, Q2, Q3 filled; Q4 M1). Change-log kinds: agree. Nothing in this record blocks Accept once the row-2 addendum is written | local run, mac a64, cbae45d, `EP9_HASH_SEED=1`; logs outside the repo: scratchpad m0m3/p2/a6logs/value-heap.log, vm-lib.log, gate-unit.log; scratchpad ci/local/m3-p1-final/x86-sandbox.log, docker-arm64.log |
