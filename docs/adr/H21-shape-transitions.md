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

**The V8-style bailout worklist. No JSC nuke bit.** JSC sets a "nuked" bit in the StructureID before it swaps a butterfly, because a JSC butterfly's size comes from its Structure (external, JSC source; not checked in this repo). An ep9 marker never reads a holder's shape to trace a child, so a nuke bit would protect nothing here, and it would take a word0 bit (one of the reserved flag bits 10-15, an H08 fill). So ep9 keeps word0 as it is and sends the rare in-place layout change to the main thread, as V8's concurrent marker does with its bailout worklist [R05].

```text
 transition kind                       mutator order (S2b on)                         marker sees
 shape only (prototype mode,           set_shape (barrier: re-grey a BLACK host)       old or new shape; both live
   SetPrototypeOf, preventExtensions)
 storage swap (SLOTS grow, dictionary  1 allocate the new cell (white)                 old storage, or new storage
   move, ELEMENTS grow or kind change)  2 fill it                                       through the re-greyed host;
                                        3 store it into the host (barrier)              cells are self-describing
                                        4 set_shape last (barrier, Release)
 in-place layout change (a kind or     notify_layout_change(cell) first: it            never sees the change half
   size change of a live cell that     excludes new marker visits (the cell goes       done: no marker is inside
   changes its size or its trace,      to the main-thread bailout worklist) and        the cell while it changes,
   a word that changes from non-Value  waits for a visit already under way, then       and concurrent markers skip
   to Value)                           returns; the change starts after it (P4).       a bailout cell
                                       The main thread visits the cell in the
                                       final stop-the-world fixpoint
 kind change that keeps the size and  barriered stores of the Value words             old or new kind: the same
   the trace (CONS or SLICED -> THIN:   (as a flatten in place)                        size and trace function
   THIN atomize, H19-Q4)
```

| # | Protocol rule (design) |
|---|---|
| P1 | Self-describing cells: the marker traces a cell from its own word0 kind and size only, never from a holder's shape. A cell kind with no Values (for example raw f64 elements, H18) is its own kind |
| P2 | word0 fields are written one atomic at a time. The mutator writes the shape id (Release under S2b) and the flags; the marker writes only gc_state. Neither side does a whole-word read-modify-write while marking runs. For the JS object kinds (16-63), the kind and the size bits of flags (bits 0-7, the in-object slot count) change only at allocation or under P4. Other flag bits are set on live cells today (is-prototype, hash present: `shape.rs`, `ordered_table.rs`); string kinds use flag bits 0 (ATOM) and 1 (TWO_BYTE), which change on live strings (`string.rs`: `set_atom`, `make_thin`) and never change a string's size. Only the mutator writes the flags half |
| P3 | A transition that moves fields writes the new storage first and the new shape last, each through the barrier. A marker that sees the old shape and the new storage, or the reverse, is safe by P1; the barrier re-greys a BLACK host, so the new storage is traced |
| P4 | An in-place layout change runs `notify_layout_change(cell)` first while marking runs, and the change starts only after it returns. When it returns, no marker is inside the cell and none can start a visit: (a) it excludes new visits (the cell goes to a main-thread bailout worklist that concurrent markers do not take, and a marker skips a cell that is on it), and (b) it waits for a visit that a marker began before the call (that marker may have read the old kind and size and be part way through the trace; putting the cell on the worklist does not stop it). If a build cannot give (a) and (b), no in-place layout change runs while concurrent marking runs: the change waits until marking ends. How markers are excluded and how the mutator waits is the Build part (H21-Q1). With marking off it is one flag test. A kind change that keeps the size and the trace function is not a layout change: CONS, SLICED and THIN are all `Fixed(32)` and share `trace_rope` (+16 and +24 are Values in all three), so THIN atomize (`make_thin`: the kind, then two barriered stores) runs while marking runs. A flatten in place that keeps the CONS kind (V8 style, H19-Q1) only stores Values, so it is a barriered store, not a layout change; a flatten that changes the size or the trace function is P4 |
| P5 | A live object's size never shrinks: slack tracking sets the capacity of new instances only (H17 slack row) |
| P6 | Weak transition children need no read barrier: the shape store of P3 re-greys the host, the final fixpoint rescans every root (CSS included), and weak transition slots are cleared only after that fixpoint (03 §4 S2b). This answers the H17 row "S2 weak transition child" |
| P7 | Compile threads read a main-thread snapshot (H30-Q4 default, Warp style), never live shapes or storage. JIT and unit code run on the mutator thread and emit P3 transitions only, through the barrier entries of the isolate table, from T1's first day |
| P8 | Shapes, descriptor arrays and transition tables are traced over their full size; unused entries hold valid Values. An append writes the entry first, then the count (Release), so no reader depends on a count |

### Freeze table (README §3.4)

| Part | What it holds | Freeze point | After the freeze point | Check |
|---|---|---|---|---|
| Design | The pick (bailout worklist, no nuke bit) and P1-P8 | M1 exit (README §3.4 row H21) | Frozen-part rules (README §3.2) | review record; Spike C and SP-Facts emit no transition, so no gate reads it before M5 |
| Build | `notify_layout_change` with its exclude-and-wait mechanism (P4, H21-Q1), the bailout worklist, the Release and fence placement of P2 and P3 in `ep9_heap` and `ep9_vm`, the emitted forms in T1 | M7 start ("functional before M7") | Frozen-part rules | the S2b gates (Validated by) |

## Rules

| Rule | Check | From |
|---|---|---|
| No trace function reads a holder's shape | review of each new kind's trace function; GC-stress with the heap verifier | M0 (as built); S2b |
| A transition that moves fields stores the storage before the shape, both through the barrier | loom model of P1-P3 with one marker thread: the marker's result is right in either store order (P1), so the order is kept for a reader of live shapes and storage, which P7 rules out today (Reopens if); V2 and V3 under S2b GC-stress | M5 (S2b) |
| An in-place layout change of a live cell goes through `notify_layout_change`, and starts only when no marker is inside the cell (P4) | a debug assert in `set_kind` (except the kind pairs that keep the size and the trace function, CONS or SLICED to THIN) and in each size change while marking runs; a loom model with one marker that is inside the cell when `notify_layout_change` starts: the change waits for the end of that visit, and a later visit skips the cell; TSan | M5 (S2b) |
| A live object's size never shrinks | heap verifier: size from word0 equals the size at allocation | M5 (slack tracking) |
| Weak transition slots are cleared only after the final fixpoint | V2 and V3 after each S2b cycle; a GC-stress test that reads a weak child and installs it during marking | M5 (S2b) |
| Emitted transitions use P3 and the barrier entries; no tier emits an in-place layout change | review of T1 and unit transition code; the S2b gate under `--force-tier=baseline` | M5 (T1); A2 |

## Consequences

- **Gains.** No word0, ShapeId or shape-space change (H8 stays as it is). Shape-only and storage-swap transitions cost what they cost under S1: the barrier fast path. Emitted code has one transition form for every tier.
- **Costs.** An in-place layout change during marking costs a wait for a marker visit already under way and a main-thread visit at the end of the cycle (P4), or the change waits until marking ends. Live objects keep slack that tracking finds later.
- **Follow-ups.** M5: S2b builds P2-P4 and the checks. H17 slack and dictionary rows, H18 elements kinds and H19-Q4 (THIN) follow P3-P5. FREEZE-M1 freezes the design part.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | A concurrent marker needs a safe protocol for shape transitions (JSC "nuke" or V8 bailout worklist); V8's concurrent marking uses a main-thread bailout worklist; JSC Riptide uses retreating-wavefront marking with a one-byte barrier compare | [../research/05-gc-design.md](../research/05-gc-design.md) |
| code | Cells are self-describing; word0 is four atomics; `set_shape` takes the barrier slow path; the only kind change of a live cell is THIN atomize (`crates/ep9_vm/src/string.rs:1001`, `make_thin`, STR2), which keeps the size and the trace function (at 12cfc61 `grep -rn 'set_kind(' crates` found only the `ep9_heap` and `ep9_value` tests) | this ADR, Context |

## Validation

- M1 exit: the design part gets the review record (README §3.1): row 1 by a GC-role agent, Codex row 2, row 3 by an agent that wrote none of it.
- M5 (S2b) to M7 start: the checks of the Rules table and the S2b gates (03 §4).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H21-Q1 | P4 requires that `notify_layout_change` excludes new marker visits of the cell and waits for a visit already under way. What mechanism does that (the Build part)? | M5 (S2b) | Today no in-place layout change is required: THIN atomize (H19-Q4) keeps the size and the trace function, so it is barriered stores and runs while marking runs (P4). If a required one appears: a handshake with the markers, or a per-visit state that a marker keeps in gc_state, the only word0 byte it writes (P2 stays; a marker never writes the flags half), with its per-visit cost added to Costs. Until one of them is built, P4's fallback holds: no in-place layout change while concurrent marking runs |
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
| 2026-10-27 | clarify (H21 is Draft) | Review fixes from the M4-P2 Claude reviewer (ADR-A0 scope): F4, THIN atomize is a kind change that keeps the size and the trace function (CONS, SLICED and THIN share `Fixed(32)` and `trace_rope`), so it is barriered stores, not P4, and runs while marking runs (the table, P4, the `set_kind` assert row, the Evidence row at the STR2 head, H21-Q1); F11, P2's size-bit rule is for the JS object kinds, and string flag bits 0-1 change on live strings; F12, a nuke bit takes a reserved word0 flag bit (an H08 fill), not a word0 change, and the loom check of the P3 row checks that either store order is safe (P1). The pick and P1-P8 do not change | M4-P2 integration fixer B |
| 2026-10-27 | clarify (H21 is Draft) | Codex round 1 (M4-P2 ADR-A0 slice, [high]; orchestrator decisions M4-P2-CODEX-R1): P4 said that putting the cell on the bailout worklist is enough, but a marker may already be inside the cell when `notify_layout_change` runs, and the worklist does not stop that visit. P4 now requires that `notify_layout_change` excludes new visits and waits for a visit already under way before the change starts, or else that no in-place layout change runs while concurrent marking runs. The mechanism stays in the Build part: the Freeze table Build row, H21-Q1 (now the mechanism question, with the lock bit or handshake default and the fallback), the transition table row, the `notify_layout_change` Rules row (a loom case with a marker inside the cell) and Costs follow. The pick and P1-P3, P5-P8 do not change; nothing is built | M4-P2 Codex-fix docs agent |
| 2026-10-27 | clarify (H21 is Draft) | M4-P2 orchestrator, from the Codex-fix checker (low): the H21-Q1 default named a word0 flag-bit lock that "a marker holds for each visit". That has a marker write the flags half, which P2 forbids, and it adds an atomic per visit that Costs does not count. The default now names a handshake or a per-visit state in gc_state (the marker's own byte), with its cost added to Costs. P2 is unchanged |
