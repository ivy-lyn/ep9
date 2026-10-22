# H09: Pages and side metadata

| Field | Value |
|---|---|
| Status | Draft (2026-10-20). S0 code built 2026-10-21: `crates/ep9_heap/src/layout.rs` (page layout), `memory.rs` (chunks, LOS), `heap.rs` (TLAB, lazy sweep). The code uses the proposed defaults of H09-Q1 to Q3: 8 B granule, 256 B lines, in-page header, LOS above 8 KiB |
| Fix by | M0 wk2 |
| Freeze point | M1 exit |
| Origin | v1#5; plan status U (unchanged) |
| Crates | `ep9_heap` (pages, bitmaps, allocation, sweep); every emitter that bakes page masks (`ep9_baseline`, `ep9_opt`) |
| Owner (plan role) | GC lead |
| Validated by | SP-GC (M0 wk2) |
| Plan sources | [02 §6 H9](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [03 §4](../plan/03-gc.md#4-stages), [03 §2](../plan/03-gc.md#2-roots) |

## Context

- CSS (H10) finds candidate pointers on native stacks. It must map an interior pointer to the start of its object.
- JIT and AOT code compute page addresses from object addresses. The mask is baked into code.
- Later stages need line marks (Immix evacuation in S3) without a layout change.
- MMTk stays an A/B backend option. Its contract needs side bits, size from header and VO lookup (03 §1).

## Decision

256 KiB aligned pages. Side bitmaps for marks, object starts (VO) and Immix lines. Heap always parsable.

```
 page (256 KiB, aligned)                    page base = addr & !(256 KiB - 1)
 +--------------------------------------+
 | page header / flags                  |   side metadata, found from the page base:
 | objects (bump allocation into lines) |     mark bitmap
 | ...                                  |     object-start (VO) bitmap  -> interior-pointer lookup
 | free lines stay parsable             |     Immix line marks          -> S3 evacuation
 +--------------------------------------+
 objects above about 8 KiB -> large-object space (LOS)
```

- S0 uses these pages for stop-the-world non-moving mark-sweep in mark-region style, lazy per-page sweep and bump allocation into free lines ([03 §4](../plan/03-gc.md#4-stages)).
- "Heap always parsable": a walker can step from object to object, because size comes from the header (H8) and free memory is a filler or is known from the VO bitmap ([R05]).

## Rules

| Rule | Check | From |
|---|---|---|
| Pages are 256 KiB and 256 KiB aligned | Unit test on the page allocator; SP-GC | M0 wk2 |
| Each allocated object sets its VO bit | Heap verifier; GC-stress | S0 |
| CSS resolves interior pointers through the VO bitmap | SP-GC probe cases S1-S3 (interior and one-past-end) | M0 wk2 |
| The heap is parsable after every allocation and sweep | Heap verifier walk; zapping of freed memory | S0 |

## Consequences

- **Gains.** The JIT bakes the masks into code. CSS gets interior-pointer lookup. Immix line marks are ready for S3 with no layout change.
- **Costs.** Side bitmaps cost memory per page. A page-size change later touches every emitter that bakes the mask, and every unit built with it.
- **Follow-ups.** H10 uses the VO bitmap. S1 adds cards for big arrays (03 §4). S3 uses line marks. The external-memory and LOS rules live in 03 §4 and §5.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | Recommendation S0: 256 KiB aligned pages; side bitmaps (marks, VO, Immix lines); LOS above about 8 KiB; free memory parsable | [05-gc-design.md](../research/05-gc-design.md) |
| [R05] | "Hard to change" items 4 (header bits and heap parsability) and 6 (page size and side-metadata scheme: the JIT bakes in the masks) | [05-gc-design.md](../research/05-gc-design.md) |
| [R05] | CSS probe: LLVM keeps interior and one-past-end pointers live, so interior lookup is required | [05-gc-design.md](../research/05-gc-design.md) |

## Validation

- **SP-GC** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk2, GC lead): spill trampolines for a64, SysV and Win64; probe cases S1-S7 as unit tests; 0 missed live objects in 10^6 random safepoints per ABI. On fail it reopens H10 and H9.
- S0 exit gate ([03 §4](../plan/03-gc.md#4-stages)): test262 pass set identical under GC-stress; verifier clean.
- M0 exit item 6: GC-stress passes every passing test.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H09-Q1 | Immix line size and the granularity of the mark and VO bitmaps (bits per 8 B or per 16 B) | M0 wk2 (`ep9_heap` S0 core) | **Filled (M0 month 2 P3, M2-P3-OQ-H09-Q1).** As built: an 8-byte granule, one mark bit and one VO bit per granule (`GRANULE` = 8, `crates/ep9_heap/src/layout.rs:31`; cells are 8-byte aligned, [R05]), and Immix lines of 256 B with one line-mark byte per line (`LINE_SIZE` = 256, `crates/ep9_heap/src/layout.rs:33`) |
| H09-Q2 | Page header layout: in-page header vs a side table indexed by page number | M0 wk2 | **Filled (M0 month 2 P3, M2-P3-OQ-H09-Q2).** As built: an in-page header for small pages. A 256 KiB small page starts with a 64 B header area (`HEADER_BYTES`, `crates/ep9_heap/src/layout.rs:42`) that holds the 12 B `PageHeader` (magic, kind, chunk index; `crates/ep9_heap/src/layout.rs:88`), then the mark bitmap (+64), the VO bitmap (+4160) and the line marks (+8256); the first cell is at offset 9472, line 37 (`FIRST_CELL_OFFSET`, `crates/ep9_heap/src/layout.rs:48-49`; const assert at `crates/ep9_heap/src/layout.rs:67`). In a small page one mask (`addr & PAGE_MASK`, `crates/ep9_heap/src/layout.rs:29`) finds all the metadata. A large object (LOS) has a `LargeHeader` only at offset 0 of its first chunk (the cell at +64, `LARGE_CELL_OFFSET`), so the mask does not find the metadata of an address in a later chunk. The heap's chunk table (`Chunk`, with `LargeTail { head }`, `crates/ep9_heap/src/memory.rs:50`) maps every address to its chunk; the conservative scan uses it to tell small pages from LOS chunks and to find a tail's head (`find_cell`, `crates/ep9_heap/src/trace.rs:357`; test `interior_pointer_into_a_large_object_tail`) |
| H09-Q3 | The exact LOS threshold. The plan says "about 8 KiB" | S0 | **Filled (M0 month 2 P3, M2-P3-OQ-H09-Q3).** As built: 8 KiB. `MAX_SMALL_SIZE` = 8 KiB (`crates/ep9_heap/src/layout.rs:38`) is the largest cell in a small page; a larger cell goes to the LOS (`crates/ep9_heap/src/heap.rs:995-996`) |
| H09-Q4 | Which emitted code uses the page mask? The ep9 barrier reads the header byte (H11), not page flags. If any AOT code bakes the mask, the unit header needs a version that covers page size. The H35 field list has none | H35 header struct (M0 wk4) | **Filled (M0 wk4, W4-H09-Q4).** No emitted code bakes the mask in v0: `PAGE_MASK` is used only by `ep9_heap` Rust code (`crates/ep9_heap/src/heap.rs:673`, `crates/ep9_heap/src/trace.rs:409`), and no machine code that ep9 emits uses it: the SP-JIT-mem test stubs and, since M0 month 2, the `spike-native` test stubs (`crates/ep9_vm/src/tests/native.rs`) and the Spike B JIT (`spikes/spike-b/src/jit/`, 11 opcodes) (grep, 2026-10-22). The unit header carries the page size as its own field instead of folding it into the object-layout version: `heap_page_log2` at offset 122 (H35 S6, H35-Q6); a mismatch gives BYTECODE. The first emitter that bakes the mask must also bump the object-layout version if it changes how the mask is used |

## Reopens if

- SP-GC fails its gate (09 §4 names H10 and H9).
- The GC-stress or verifier gate of S0 fails because the heap is not parsable.
- G7 or the S3 RSS gate shows that side metadata costs too much memory. The plan names no such trigger; record it as a reopen only with data.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6, 03 §2 and §4 (M0 wk1 list) | ADR agent |
| 2026-10-21 | clarify | Status: S0 code built in `ep9_heap` with the H09-Q1 to Q3 defaults (8 B granule bitmaps, 256 B line marks, in-page header with first cell at offset 9472, LOS above 8 KiB in its own 256 KiB chunk run). The questions stay open until the review record | heap agent |
| 2026-10-21 | fill | H09-Q4: no emitted code bakes the page mask in v0; the H35 unit header carries the page size as its own field (`heap_page_log2`), not under the object-layout version. Decision W4-H09-Q4 | P1-AOTU agent |
| 2026-10-21 | clarify | 2 `path:line` references moved to the code at `479b1cd` (P4-DOCS drift script, [I15]); the `PAGE_MASK` use in `heap.rs` changed form and is pinned to its new line | P4-DOCS agent |
| 2026-10-22 | clarify | H09-Q4 fill text: Spike B (M0 month 2) emits code but bakes no page mask, so "the only machine code ep9 emits is the SP-JIT-mem test stubs" and "the first emitter that bakes the mask (Spike B)" were out of date; the `spike-native` stubs and the Spike B JIT are named, and no emitter uses `PAGE_MASK`. No check changes | P3-ACCEPT-2 |
| 2026-10-22 | fill | H09-Q1 filled as built: 8-byte granule for the mark and VO bitmaps, `LINE_SIZE` 256 (`layout.rs`). Every check keeps its result (M2-P3-OQ-H09-Q1) | P3 integration fixer |
| 2026-10-22 | fill | H09-Q2 filled as built: in-page header, first cell at offset 9472 (`layout.rs`). Every check keeps its result (M2-P3-OQ-H09-Q2) | P3 integration fixer |
| 2026-10-22 | fill | H09-Q3 filled as built: `MAX_SMALL_SIZE` 8 KiB is the LOS threshold (`layout.rs`). Every check keeps its result (M2-P3-OQ-H09-Q3) | P3 integration fixer |
| 2026-10-22 | clarify | H09-Q2 fill text as built: the 64 B header area holds a 12 B `PageHeader`; the one-mask lookup holds for small pages only, and a LOS tail chunk finds its head through the chunk table (Codex C2, M2-P4-B2-ROW2). No check changes | P4-ACCEPT-3 |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P3-ACCEPT-2 (did not write the ADR) | Decision = the 02 §6 H9 row. Checked against `ep9_heap` at e719ffa: `PAGE_SIZE` 256 KiB, `GRANULE` 8, `LINE_SIZE` 256, the first cell at 9472 (const assert, `crates/ep9_heap/src/layout.rs:67`), `MAX_SMALL_SIZE` 8 KiB (larger cells go to the LOS); the verifier walks cells in address order from the VO bitmap (`verify.rs`); the CSS interior and one-past-end tests (`src/tests/css.rs`). R1: the H09-Q4 fill said the only emitted code is the SP-JIT-mem stubs and named Spike B as the first emitter to bake the mask; Spike B emits code but uses no page mask (`PAGE_MASK` appears only in `ep9_heap`) (fixed, clarify). Open and due in M0, not filled (reported): H09-Q1, H09-Q2 and H09-Q3 (Needed by M0 wk2 or S0); the code uses their proposed defaults (8 B granule, 256 B lines, in-page header, LOS above 8 KiB; Status line). Change-log kinds: agree | c68971a |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | Two runs. (1) M2-P3-B2-ROW2 ("ADR docs" slice of the P3 merge, base 66f08cd): no finding on H09. (2) The P4 pre-step run on every P3 change of H01, H06, H09, H10, H11, H28 and H36, fc368e1 and 077d1fe included (base f8746a4): 3 findings. On H09, C2 (medium): the H09-Q2 fill calls the header a 64 B `PageHeader` (the struct is 12 B in a 64 B area) and says one mask finds all the metadata, which is false for an address in a LOS tail chunk | decisions-log M2-P3-B2-ROW2; M2-P4-B2-ROW2; Codex text outside the repo: scratchpad review/m2p3-doc.codex.txt, review/m2p4-b2.codex.txt |
| 3 Verifier | P4-ACCEPT-3 (wrote none of the rows it checks) | Checked at 0c4c70e. C2 reproduced: `PageHeader` is u32 + u8 + 3 pad + u32 = 12 B (`crates/ep9_heap/src/layout.rs:88`) inside `HEADER_BYTES` = 64; `find_cell` sends a `LargeTail { head }` chunk to its head's `LargeHeader` (`crates/ep9_heap/src/trace.rs:357`), and the test `interior_pointer_into_a_large_object_tail` passes. Real, category (b) (made by the fc368e1 fill). Fixed (clarify row; the small-page numbers stand). Rows written by others, fc368e1: H09-Q1 (`GRANULE` 8 and `LINE_SIZE` 256, `crates/ep9_heap/src/layout.rs:31`, :33: true); H09-Q2 (offsets: 64 + 512 words x 8 = 4160, + 4096 = 8256, + 1024 line bytes rounded up to 256 = 9472 = line 37, as the const assert says: true; header size and LOS: C2); H09-Q3 (`MAX_SMALL_SIZE` 8 KiB, `crates/ep9_heap/src/layout.rs:38`; `alloc_in` sends a larger cell to `alloc_large`, `crates/ep9_heap/src/heap.rs:995-996`: true); kind fill for the three: agree (as-built values, no check changes). The P3-ACCEPT-2 H09-Q4 clarify: `PAGE_MASK` appears only in `ep9_heap` (grep of `crates` and `spikes` at 0c4c70e): true. 077d1fe moved the H09-Q4 ref of the `PAGE_MASK` use from line 619 to `crates/ep9_heap/src/heap.rs:673`, the `addr & PAGE_MASK` of the VO-bit store: true. Change-log kinds: agree | branch m0m2/p4-accept-3; local run, mac a64, 0c4c70e |
| 4 Data | from the "Validated by" line | SP-GC: `bash spikes/sp-gc/gate.sh` PASS on mac a64 (probes S1-S7; 10^6 safepoints x 3 seeds on aapcs64 with 0 misses; the 3 negative controls miss as they must; the Win64 build and its save and SEH codes match); the Rosetta x86_64 step SKIP (Rosetta is hung on this host, M2-P3-ROSETTA). `cargo test -p ep9_heap --locked`: 84 pass. Missing: the GC-stress test262 run is the CI test262 job (linux-x64); the SysV and Win64 runs are the CI spikes job. P4 refresh (0c4c70e): `cargo test -p ep9_heap --locked` 87 pass (85 unit tests and 2 doctests), `interior_pointer_into_a_large_object_tail` among them. The real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3), with the linux-x64 GC-stress test262 step and the spike gates on 4 runners | local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/sp-gc.log, heap.log; P4: scratchpad m0m2/accept3/logs/heap.log |
