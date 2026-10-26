# H21: Shape transitions under concurrent readers

| Field | Value |
|---|---|
| Status | Draft (2026-10-27): the protocol is picked (a V8-style bailout worklist, no JSC nuke bit) and written as a design. Nothing is built: the GC is S0 (stop-the-world), and S2b concurrent marking comes in M5 |
| Fix by | ADR M1; built M5; functional before M7 |
| Freeze point | Two parts (the Freeze table below, README §3.4): the design at M1 exit; the build at M7 start ("functional before M7") |
| Origin | v1#17; plan status U |
| Crates | `ep9_heap` (word0 stores, the barrier slow paths, the bailout worklist, the S2b marker), `ep9_vm` (`shape.rs`, `props.rs`, `object.rs`, `string.rs`: the transitions); later `ep9_baseline`, `ep9_opt`, `ep9_masm` (emitted transitions) |
| Owner (plan role) | VM lead (author); GC lead (row 1 reviewer, S2b) |
| Validated by | The S2b gates of 03 §4: before M7, GC-stress with the verifiers V2 and V3 after each cycle, 0 verifier failures on test262 and v8-v7 under `--force-tier=baseline`; TSan clean over 1,000 CPU-hours of stress; loom tests |
| Plan sources | [02 §6 H21](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [03 §4 S2b](../plan/03-gc.md#4-stages), [H17](H17-object-layout.md), [H08](H08-object-header-and-shapeid.md), [H11](H11-barrier-and-atomics.md) |

## Context

- From S2b (M5) a helper thread marks while the mutator runs (03 §4: retreating wavefront, white allocation, a stop-the-world fixpoint that rescans all roots, no weak read barrier). A transition then changes a shape, a storage cell or a kind byte while another thread may read it.
- The plan row asks the ADR to pick "the JSC 'nuke' protocol or the V8 bailout worklist", and "all tiers and AOT code use it". The risk it names: T1 emits transitions before S2b exists [R05].
- In ep9 every cell describes itself. Its kind byte picks its trace function, and its size comes from the cell itself: a JS object's size from its own word0 (kind + flags), and every word after word0 of a JS object is a Value, so one trace function per kind visits them all; no trace function reads the holder's shape (`crates/ep9_vm/src/object.rs:11`, `crates/ep9_vm/src/object.rs:39-41`). A SLOTS cell has its own capacity (`crates/ep9_vm/src/props.rs:15`).
- word0 is four separate atomics: shape id (u32), gc_state (u8), kind (u8), flags (u16) (`crates/ep9_value/src/header.rs:47-52`). A transition writes the shape id through `set_shape`, which takes the barrier slow path when the host is marked (`crates/ep9_heap/src/barrier.rs:167-173`).
- Depends on: H8 (word0, ShapeId), H11 (barrier v2 and its S2 slow path), H17 (transitions, slack, dictionary mode, the S2 weak transition child row), H18 (elements reallocation), H19 (in-place string changes). Depended on by: H17 and H18 (their transitions follow it), T1, T2 and unit code (emitted transitions), the S2b build (03 §4).

## Decision

**The V8-style bailout worklist. No JSC nuke bit.** JSC sets a "nuked" bit in the StructureID before it swaps a butterfly, because a JSC butterfly's size comes from its Structure (external, JSC source; not checked in this repo). An ep9 marker never reads a holder's shape to trace a child, so a nuke bit would protect nothing here, and it would need a word0 or ShapeId change (H8). So ep9 keeps word0 as it is and sends the rare in-place layout change to the main thread, as V8's concurrent marker does with its bailout worklist [R05].

```text
 transition kind                       mutator order (S2b on)                         marker sees
 shape only (prototype mode,           set_shape (barrier: re-grey a BLACK host)       old or new shape; both live
   SetPrototypeOf, preventExtensions)
 storage swap (SLOTS grow, dictionary  1 allocate the new cell (white)                 old storage, or new storage
   move, ELEMENTS grow or kind change)  2 fill it                                       through the re-greyed host;
                                        3 store it into the host (barrier)              cells are self-describing
                                        4 set_shape last (barrier, Release)
 in-place layout change (a kind or     notify_layout_change(cell) first: the cell      never sees the change half
   size change of a live cell, a word   goes to the main-thread bailout worklist;      done: concurrent markers
   that changes from non-Value to       the main thread visits it in the final         skip a bailout cell
   Value: THIN atomize, H19-Q4)         stop-the-world fixpoint
```

| # | Protocol rule (design) |
|---|---|
| P1 | Self-describing cells: the marker traces a cell from its own word0 kind and size only, never from a holder's shape. A cell kind with no Values (for example raw f64 elements, H18) is its own kind |
| P2 | word0 fields are written one atomic at a time. The mutator writes the shape id (Release under S2b) and the flags; the marker writes only gc_state. Neither side does a whole-word read-modify-write while marking runs. The kind and the size bits of flags (bits 0-7, the in-object slot count) change only at allocation or under P4. Other flag bits are set on live cells today (is-prototype, hash present, atom: `shape.rs`, `ordered_table.rs`, `string.rs`); they do not change the size, and only the mutator writes the flags half |
| P3 | A transition that moves fields writes the new storage first and the new shape last, each through the barrier. A marker that sees the old shape and the new storage, or the reverse, is safe by P1; the barrier re-greys a BLACK host, so the new storage is traced |
| P4 | An in-place layout change runs `notify_layout_change(cell)` first while marking runs: the cell goes to a main-thread bailout worklist that concurrent markers do not take. With marking off it is one flag test. An optional change (THIN atomize) may be skipped while marking runs (H21-Q1). A flatten in place that keeps the CONS kind (V8 style, H19-Q1) only stores Values, so it is a barriered store, not a layout change; a flatten that changes the kind is P4 |
| P5 | A live object's size never shrinks: slack tracking sets the capacity of new instances only (H17 slack row) |
| P6 | Weak transition children need no read barrier: the shape store of P3 re-greys the host, the final fixpoint rescans every root (CSS included), and weak transition slots are cleared only after that fixpoint (03 §4 S2b). This answers the H17 row "S2 weak transition child" |
| P7 | Compile threads read a main-thread snapshot (H30-Q4 default, Warp style), never live shapes or storage. JIT and unit code run on the mutator thread and emit P3 transitions only, through the barrier entries of the isolate table, from T1's first day |
| P8 | Shapes, descriptor arrays and transition tables are traced over their full size; unused entries hold valid Values. An append writes the entry first, then the count (Release), so no reader depends on a count |

### Freeze table (README §3.4)

| Part | What it holds | Freeze point | After the freeze point | Check |
|---|---|---|---|---|
| Design | The pick (bailout worklist, no nuke bit) and P1-P8 | M1 exit (README §3.4 row H21) | Frozen-part rules (README §3.2) | review record; Spike C and SP-Facts emit no transition, so no gate reads it before M5 |
| Build | `notify_layout_change`, the bailout worklist, the Release and fence placement of P2 and P3 in `ep9_heap` and `ep9_vm`, the emitted forms in T1 | M7 start ("functional before M7") | Frozen-part rules | the S2b gates (Validated by) |

## Rules

| Rule | Check | From |
|---|---|---|
| No trace function reads a holder's shape | review of each new kind's trace function; GC-stress with the heap verifier | M0 (as built); S2b |
| A transition that moves fields stores the storage before the shape, both through the barrier | loom model of P3 with one marker thread; V2 and V3 under S2b GC-stress | M5 (S2b) |
| An in-place layout change of a live cell goes through `notify_layout_change` | a debug assert in `set_kind` and in each size change while marking runs; TSan | M5 (S2b) |
| A live object's size never shrinks | heap verifier: size from word0 equals the size at allocation | M5 (slack tracking) |
| Weak transition slots are cleared only after the final fixpoint | V2 and V3 after each S2b cycle; a GC-stress test that reads a weak child and installs it during marking | M5 (S2b) |
| Emitted transitions use P3 and the barrier entries; no tier emits an in-place layout change | review of T1 and unit transition code; the S2b gate under `--force-tier=baseline` | M5 (T1); A2 |

## Consequences

- **Gains.** No word0, ShapeId or shape-space change (H8 stays as it is). Shape-only and storage-swap transitions cost what they cost under S1: the barrier fast path. Emitted code has one transition form for every tier.
- **Costs.** An in-place layout change during marking costs a main-thread visit at the end of the cycle, or the change is skipped. Live objects keep slack that tracking finds later.
- **Follow-ups.** M5: S2b builds P2-P4 and the checks. H17 slack and dictionary rows, H18 elements kinds and H19-Q4 (THIN) follow P3-P5. FREEZE-M1 freezes the design part.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | A concurrent marker needs a safe protocol for shape transitions (JSC "nuke" or V8 bailout worklist); V8's concurrent marking uses a main-thread bailout worklist; JSC Riptide uses retreating-wavefront marking with a one-byte barrier compare | [../research/05-gc-design.md](../research/05-gc-design.md) |
| code | Cells are self-describing; word0 is four atomics; `set_shape` takes the barrier slow path; no code changes a live cell's kind today (`grep -rn 'set_kind(' crates` at 12cfc61 finds only the `ep9_heap` and `ep9_value` tests) | this ADR, Context |

## Validation

- M1 exit: the design part gets the review record (README §3.1): row 1 by a GC-role agent, Codex row 2, row 3 by an agent that wrote none of it.
- M5 (S2b) to M7 start: the checks of the Rules table and the S2b gates (03 §4).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H21-Q1 | A required in-place layout change while a concurrent marker may be inside that cell: how does the mutator wait for the visit to end? | M5 (S2b) | Today no in-place change is required: THIN atomize (H19-Q4) is skipped while marking runs. If a required one appears: a cell lock bit in the reserved word0 flag bits 10-15 (an H08 fill), else a handshake with the markers |
| H21-Q2 | Does any tier need to emit an in-place layout change? | M5 (T1) | No: emitted code makes only P3 transitions and calls the barrier entries |
| H21-Q3 | Does H21 need a shape-space or word0 change (plan v9 §12.1)? | M1 exit | No: the decision keeps word0 and the ShapeId rule of H8 as they are |
| H21-Q4 | The size of the loom model (threads, interleavings) and the TSan stress budget per CI run | M5 | One mutator and one marker thread per model; the 1,000 CPU-hours of 03 §4 spread over nightly runs |

## Reopens if

- A marker must read a holder's shape to trace a child (P1 breaks). Then a nuke-style bit becomes needed.
- A compile thread reads live shapes or storage (H30-Q4 changes).
- A required in-place layout change lands on a hot path, or the S2b gates find a race that P1-P8 do not cover.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-27 | draft | First draft (plan v9 §4.1 task 1, JA-15): the bailout worklist over the nuke bit, the transition table, P1-P8, the Freeze table (design at M1 exit, build at M7 start), Q1-Q4. README §5 moves H21 from Not started to Draft (V9-A22) | M4-P2 ADR-A0 (VM lead role) |
