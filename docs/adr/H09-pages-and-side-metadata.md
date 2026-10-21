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
| H09-Q1 | Immix line size and the granularity of the mark and VO bitmaps (bits per 8 B or per 16 B) | M0 wk2 (`ep9_heap` S0 core) | 8-byte granularity, as objects are 8-byte aligned ([R05]) |
| H09-Q2 | Page header layout: in-page header vs a side table indexed by page number | M0 wk2 | Decide in S0 core |
| H09-Q3 | The exact LOS threshold. The plan says "about 8 KiB" | S0 | 8 KiB |
| H09-Q4 | Which emitted code uses the page mask? The ep9 barrier reads the header byte (H11), not page flags. If any AOT code bakes the mask, the unit header needs a version that covers page size. The H35 field list has none | H35 header struct (M0 wk4) | **Filled (M0 wk4, W4-H09-Q4).** No emitted code bakes the mask in v0: `PAGE_MASK` is used only by `ep9_heap` Rust code (`crates/ep9_heap/src/heap.rs:619`, `crates/ep9_heap/src/trace.rs:409`), and the only machine code ep9 emits is the SP-JIT-mem test stubs. The unit header carries the page size as its own field instead of folding it into the object-layout version: `heap_page_log2` at offset 122 (H35 S6, H35-Q6); a mismatch gives BYTECODE. The first emitter that bakes the mask (Spike B) must also bump the object-layout version if it changes how the mask is used |

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
