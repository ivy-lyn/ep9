# H11: Barrier and atomics

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P4-ACCEPT; review record complete). Was: Draft (2026-10-20). Decided 2026-10-20 (gc-barrier decision, critic additions). Accept needs the review record ([README §3.1](README.md#31-review-record-p-1)). Code 2026-10-21: barrier v2 API and S0 slow path in `crates/ep9_heap/src/barrier.rs`; model test in `src/tests/model.rs`; fast-path check `crates/ep9_heap/disasm-barrier.sh` |
| Fix by | API M0; slow paths S1/S2; unit field A2 |
| Freeze point | M1 exit (API, fast-path shape, gc_state values, emitter rule); slow paths may change without a new barrier version |
| Origin | v1#7; plan status E (extended: AOT code kinds, barrier version) |
| Crates | `ep9_heap` (barrier API, slow paths, verifiers, root-source registry), `ep9_value` (gc_state byte), `ep9_vm` (T0 handlers, Rust setters), `ep9_baseline`, `ep9_opt` (emitted fast path, elision), `ep9_unit` (barrier version) |
| Owner (plan role) | GC lead; VM lead for SP-Value |
| Validated by | SP-Value (M0 wk2); barrier model test (M0 wk2); verifier V1 (S1); verifiers V2, V3 and loom (S2b) |
| Plan sources | [02 §6 H11](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [03 §3](../plan/03-gc.md#3-barrier-one-fast-path-in-every-tier-and-code-kind), [03 §4](../plan/03-gc.md#4-stages), [03 §7](../plan/03-gc.md#7-gc-test-matrix-per-mode) |

## Context

- One store barrier must serve sticky-mark generations (S1) and concurrent marking (S2) without a rewrite of every store site.
- The first draft paired the JSC fast path (`is_cell(new)`) with an SATB slow path. That pair is unsound: `host.f = 5` over a cell skips the snapshot of the old cell (SP-Value, H11-Q4). An old-value filter is unsound for S1: it misses `host.f = young` over an int.
- AOT code bakes in the barrier, as JIT code does. HotSpot Leyden rejects AOT code when GC barriers differ [R11].
- Concurrent marking (S2b) reads fields while the mutator writes them. Risk RK12: data races in concurrent marking.

## Decision

ep9 uses the JSC Riptide barrier [J1][J2]. One object-granularity barrier serves S1 and S2. S2 is incremental update with a retreating wavefront. It is **not** SATB. The full rule set (state and threshold tables, slow path per stage, marker rule, termination, allocation colour, weak slots, roots, elision E1-E3, emitter rule, barrier version) is [03 §3](../plan/03-gc.md#3-barrier-one-fast-path-in-every-tier-and-code-kind). Summary:

```
store(host, slot, v):
    if is_cell(v) && host.gc_state <= isolate.barrier_threshold { barrier_slow(host, slot, v) }  // slow path stores
    else { slot.store(v, Relaxed) }
```

| Item | Decision |
|---|---|
| Filter | New value only |
| Order | Check first; the slow path does `store (Release); fence (SeqCst); load gc_state` (Riptide post-store order) |
| Emitter rule | No GC point between the check and the store. A loaded threshold or gc_state is never reused across a GC point |
| gc_state | BLACK = 1, WHITE = 2, GREY = 3 |
| Threshold | OFF = 0 (S0), GEN = 1 (S1), MARKING = 0xFF (S2 marking) |
| S1 logs | The BLACK host (-> GREY); a 512-byte card for large arrays |
| S2 logs | The BLACK, marked host (-> GREY) for a revisit; a BLACK, marked large array also dirties its card and is pushed once (card-only revisit); never the old value |
| Marker | Set BLACK, SeqCst fence, read fields. Field loads Acquire, or an acquire fence before children are published to a shared deque |
| Cycle start | A full cycle empties the S1 remembered-set buffer |
| Termination | Stop-the-world fixpoint. TLAB retired first (VO bits). Rescans every root source: CSS, VM stack, `Global<T>`, `ScratchRoots`, unit RW link cells, embedder roots, mutator mark stack, dirty cards of marked large arrays, weak and ephemeron constraints. Space-time scheduler |
| Allocation | WHITE in every stage |
| Weak reads | No read barrier |
| Weak slots | Cleared at every minor GC (S1) and every S2 end phase when the referent is unmarked |
| Unit RW cells, roots | No write barrier; rescanned |
| Elision | E1 non-cell value; E2 fresh, unescaped host with no GC point since allocation; E3 roots |
| Barrier version | 2 |

### Options considered

| Option | Verdict | Why |
|---|---|---|
| A. Retreating wavefront (JSC Riptide) | Chosen | Same architecture in production; one barrier for both; no weak read barriers; non-cell stores free; no floating garbage |
| B. Object-logging SATB (MMTk SATBBarrier + ObjectBarrier, LXR idea) | Rejected | Weak read barrier at every weak-to-strong site (Rust tables, transition ICs, JIT and AOT weak constants); non-cell stores cannot be elided; sticky marks need black-but-new handling; no production precedent with sticky marks |
| C. Slot SATB pre + card post (G1, Shenandoah, Hermes) | Rejected | Two checks per store, plus the SATB weak-read burden |
| D. Dijkstra insertion, no host check (V8) | Rejected | Marks floating garbage that sticky marks then promote; still rescans roots |
| E. Go hybrid (Yuasa + Dijkstra) | Rejected | Solves STW scans of many goroutine stacks; ep9 has one stack per isolate (D14); needs allocate-black |
| F. Field logging (LXR) | Rejected | Per-field unlog bits; built for RC; SATB burden |

## Rules

| Rule | Check | From |
|---|---|---|
| Every heap reference store goes through the barrier API or an elision rule E1-E3 | API in `ep9_heap`; emitter elision analysis; V1, V2, V3 | M0 (API) |
| The filter reads the new value only; the slow path does the store | Barrier model test; SP-Value disassembly | M0 wk2 |
| No GC point between the check and the store; no threshold or gc_state reuse across a GC point | Emitter review; barrier model test | M5; A2 |
| Every GC-visible field access is a Relaxed atomic. The S2 slow path adds a Release store and a SeqCst fence. The marker sets BLACK, fences, then reads fields; field loads are Acquire (or an acquire fence before publishing children) | Review; loom with 2 markers + 1 mutator; TSan from S2 | M0 rule; S2 |
| A ShapeId store into word0 goes through `barrier_slow_shape` when the check fires | Review; V2 | M1 |
| `barrier_slow` never allocates on the GC heap and never reaches a safepoint | Review; debug no-GC scope | M1 |
| Every root source can be rescanned with the world stopped, dirty LOS cards included | Root-source registry in `ep9_heap` (H10); V3 | M5 |
| Every S2 pause retires the TLAB before CSS | Review; GC-stress | M5 |
| Every minor GC clears weak slots whose referent is an unmarked young object | V1 run; GC-stress with weak IC cells, transitions, WeakRef | M1 |
| No barrier clustering (one barrier for many stores) | Emitter review; negative control | M5 |
| Fast path = 4 instructions plus the cell test on aarch64 | SP-Value disassembly | M0 wk2 |
| One fast-path shape in every tier and code kind | Shared emitter per crate; verifiers under `--force-tier` and `--force-mode` | M5; M8 exit |
| A unit with another barrier version runs its bytecode, never crashes | Unit golden files; header-mutation tests | A2 |

## Consequences

- **Gains.** One barrier and one header byte for S1 and S2: 4 instructions plus the cell test on aarch64. Non-cell stores need no barrier. No weak read barriers anywhere. No floating garbage. A concurrent minor GC stays possible later with the same barrier.
- **Costs.**
  - While marking runs, every cell store calls the slow path and pays two fences (Riptide: about 5% with the re-check inlined; ep9 starts out of line, H11-Q5). The marker pays one fence per visited object.
  - Termination is not bounded by construction. A space-time scheduler paces the mutator, and the S2b p99 gate measures it.
  - Every root source must support a rescan in the pause. Dirty LOS cards add card work to the pause; the card-only revisit drains most of it before.
  - An old LOS array stays BLACK, so every cell store into it calls the slow path out of line in S1. S1 records this call rate.
- **Follow-ups.** H21 (shape transitions under concurrent readers). S3 records candidate slots in the same slow path. D14 = shared heap would need an ADR amendment of H11, H22 and H23 (11-decisions).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | JSC Riptide: one header-byte compare for generational and concurrent paths; relaxed-atomic rule; the fast-path shape is the hard-to-change part. Its SATB slow-path recommendation is superseded by this ADR | [05-gc-design.md](../research/05-gc-design.md) |
| [R11] | JEP 544: AOT code needs the same GC barriers | [11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [J1] | Riptide: one barrier for generations and concurrency; conditional fence through the threshold; about 5% during GC, 0% otherwise; JIT elides non-cell and new-object barriers; revisit deferral; space-time scheduler; advancing wavefront fits generations badly; object granularity, cards were slower | https://webkit.org/blog/7122/introducing-riptide-webkits-retreating-wavefront-concurrent-garbage-collector/ |
| [J2] | WebKit main bdf0502 (2026-10-20): heap/CellState.h:31-52; heap/HeapInlines.h:87-111; heap/Heap.cpp:1154-1210, 1370, 2189-2199, 2398-2449, 2705-2709; heap/SlotVisitor.cpp:386-388; heap/Collector.cpp:500-573, 997-1016; heap/MarkedBlock.h:315-350; dfg/DFGStoreBarrierInsertionPhase.cpp:488-515; runtime/JSCellInlines.h:240-259; jit/GPRInfo.h:249-250 | https://github.com/WebKit/WebKit/tree/main/Source/JavaScriptCore |
| [S1] | SATB needs weak-read barriers (Hermes notes that weak roots used only for comparisons need none) | Hermes doc/Hades.md:243-258; mmtk-core src/plan/concurrent/barrier.rs:143-160; OpenJDK g1BarrierSet.inline.hpp:83-95 |
| [S2] | One log can serve remsets and SATB (object or field logging) | mmtk-core src/plan/barriers.rs:197-259, 356-406; LXR (arXiv 2210.17175) §3.4 |
| [S3] | V8 marks the new value, skips the host check to avoid a fence, re-scans roots at finalization | https://v8.dev/blog/concurrent-marking ; v8 src/heap/heap-write-barrier-inl.h:28-80 |
| [S4] | Go hybrid barrier exists to avoid STW rescans of many goroutine stacks; it needs allocate-black | https://github.com/golang/proposal/blob/master/design/17503-eliminate-rescan.md |
| [M] | Exhaustive interleaving model: decided form passes 5 scenarios; 6 rejected forms each lose an object. Critic model: the first decided card text (dirty a card, no push, cards not a root source) loses an object; a card rescan or a push fixes it | `ep9_heap` barrier model test (`crates/ep9_heap/src/tests/model.rs`); [model.py](../research/artifacts/decide/gc-barrier/model/model.py) and [los_card_model.py](../research/artifacts/decide/critic/los_card_model.py) (copies of the 2026-10-20 scratch files, re-run 2026-10-21) |

## Validation

- **SP-Value** (M0 wk2): fast path 4 instructions plus the cell test on aarch64 (`ep9_probe_barrier`). On fail it reopens H6 and H11.
- **Barrier model test** (M0 wk2): exhaustive interleavings, 6 scenarios (move, alloc, reg, escape, weak, los); the decided form passes; the rejected forms fail (including the "card without rescan or push" form).
- **S1** (M1): V1 remembered-set verifier before every minor GC under GC-stress; negative controls fail.
- **S2b** (before M7): V2 closure and V3 re-mark verifiers after every cycle under concurrent GC-stress, `--force-tier=baseline`; by M8 exit also `opt` and the forced AOT modes; loom on the fence pairs with 2 markers and 1 mutator, the re-whitening CAS and the card clear/dirty pair; TSan clean over 1,000 CPU-hours. Negative control "no card rescan at termination" must make V2 fail.
- **A2** / every PR touching units: barrier-version mismatch gives bytecode fallback ([03 §7](../plan/03-gc.md#7-gc-test-matrix-per-mode)).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H11-Q1 | Instruction budget on x64 | Answered 2026-10-20 | SysV and Win64: 3 (cell test) + 4 + store (rustc 1.99 -O3). Record in SP-Value; no gate |
| H11-Q2 | Where the threshold lives | M0 (API) | **Filled (M0 month 2 P3, M2-P3-OQ-H11-Q2).** As built: a byte at heap offset 0 (`Heap::BARRIER_THRESHOLD_OFFSET` = 0, `crates/ep9_heap/src/heap.rs:330`; the threshold is the first field of `BarrierState`, `crates/ep9_heap/src/barrier.rs:80`). T0 reaches it through `Cx`: `Cx::rt` points at the `Runtime`, and every barriered store passes `Runtime::heap`'s `Heap::barrier()` to `Slot::set` (`object::wr`, `crates/ep9_vm/src/object.rs:335-340`). Tests `decided_constants` and `heap_field_offsets_hold_at_run_time` (`crates/ep9_heap/src/tests/barrier.rs`) |
| H11-Q3 | How the barrier version is numbered | H35 header struct (M0 wk4) | **Filled (M0 wk4, W4-H11-Q3).** One counter, `BARRIER_VERSION` (`crates/ep9_heap/src/barrier.rs:50`), bumped by review when anything its doc comment lists changes. Now 2. The H35 unit header carries it as a u32 at offset 100; a mismatch gives BYTECODE (`crates/ep9_unit`, tests `every_header_field_mutation_gives_its_listed_outcome`) |
| H11-Q4 | Closed 2026-10-20. The cell test reads the NEW value. S2 is retreating wavefront, not SATB, so a store that deletes a reference needs no barrier. Old only is unsound for S1 and S2. Old OR new is sound but not needed (one more load when the host fires). Evidence: [J1], [J2] HeapInlines.h:87-111, [M] | — | Closed |
| H11-Q5 | Inline `fence; re-check` in the JIT and AOT cold stub (Riptide does) | S2b (M5) | Out of line first. Inline if the mutator slows down more than 10% while marking runs (barrier-version bump) |

## Reopens if

- SP-Value fails the barrier instruction budget (09 §4 names H6 and H11).
- The S2b p99 pause gate fails after scheduler tuning because the fixpoint does not converge (then evaluate option B with the same header byte).
- Mutator slowdown while marking stays above 10% on the JetStream 3 JS subset after the fenced re-check is inlined (H11-Q5).
- The S2b verifiers, loom or TSan find a race that the fence rules cannot fix.
- V1, V2 or V3 finds a miss that rules E1-E3, the emitter rule and the root registry cannot explain.
- D14 changes to a shared heap (11-decisions "Affects": H11, H22, H23).
- A moving nursery is added.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6 and 03 §3-§4 (M0 wk1 list) | ADR agent |
| 2026-10-20 | draft | Add H11-Q4 (old vs new value in the cell test) with the SP-Value evidence | fix agent |
| 2026-10-20 | rewrite | JSC Riptide barrier for S1-S3; S2 is retreating wavefront, not SATB; new-value filter; slow path does the store; white allocation; stop-the-world root rescan; elision E1-E3; verifiers V1-V3; barrier version 2. H11-Q4 closed, H11-Q1 answered, H11-Q5 added | gc-barrier agent |
| 2026-10-20 | rewrite | Critic additions: dirty LOS cards are a termination root source and a BLACK LOS host is pushed once (an exhaustive model lost an object without this); emitter rule (no GC point between check and store, no threshold reuse); weak slots cleared at every minor GC; marker Acquire loads with stealing markers; TLAB retired before each CSS; full cycle empties the S1 buffer; model test gains the `los` scenario and a fifth negative control | apply agent |
| 2026-10-21 | clarify | Status: barrier v2 built (`BARRIER_VERSION` = 2, gc_state BLACK=1 WHITE=2 GREY=3, thresholds OFF=0 GEN=1 MARKING=0xFF, `Slot::set` new-value filter with the store in the slow path, `Slot::init`, shape-install barrier). Model test ported to Rust with the `los` scenario and 5 negative controls. `disasm-barrier.sh` checks cell test + 4 header-check instructions + 1 store on aarch64 and x86_64 | heap agent |
| 2026-10-21 | clarify | Evidence [M] names the ported model test file and links the repo copies of the two models in `docs/research/artifacts/decide/` instead of the session scratch paths | docs-sync agent |
| 2026-10-21 | fill | H11-Q3: one counter (`BARRIER_VERSION` = 2), carried in the H35 unit header at offset 100; a mismatch gives BYTECODE. Decision W4-H11-Q3 | P1-AOTU agent |
| 2026-10-22 | fill | H11-Q2 filled as built: the threshold is a byte at heap offset 0 (`Heap::BARRIER_THRESHOLD_OFFSET`), which T0 reaches through `Cx`. Every check keeps its result (M2-P3-OQ-H11-Q2) | P3 integration fixer |
| 2026-10-22 | clarify | 3 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P4-ACCEPT). The §3.1 review record is complete: row 1 P3-ACCEPT-2, row 2 Codex (M2-P3-B2-ROW2 and the P4 pre-step run (M2-P4-B2-ROW2)), row 3 P4-ACCEPT-3 (and the P4 review checkers for the RF1 rows), row 4 data. The orchestrator did not write the rows it approves. No check changes | orchestrator |
| 2026-10-23 | clarify (proposed; Accepted path) | 2 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it; the orchestrator approves | M3-P0 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P3-ACCEPT-2 (did not write the ADR) | Decision = the 02 §6 H11 row and 03 §3 (the Riptide barrier, decided 2026-10-20). Checked against `ep9_heap` at e719ffa: `BARRIER_VERSION` = 2 (`crates/ep9_heap/src/barrier.rs:50`); gc_state BLACK=1, WHITE=2, GREY=3; the threshold byte at a fixed heap offset (`Heap::BARRIER_THRESHOLD_OFFSET` = 0, `crates/ep9_heap/src/heap.rs:330`); the model test (6 scenarios; the decided form passes; the 5 negative controls of 03 §7 each lose an object); `disasm-barrier.sh`; the unit header barrier field (`every_header_field_mutation_gives_its_listed_outcome`). No defect found. Open and due in M0, not filled (reported): H11-Q2 (where the threshold lives; Needed by M0 API): the code answers it (a byte at heap offset 0). Change-log kinds: agree (the second 2026-10-20 "draft" row added Q4 before the first review; TEMPLATE lists the kind draft) | review at e719ffa (no text change) |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | No finding on H11 in either run: M2-P3-B2-ROW2 ("ADR docs" slice of the P3 merge, base 66f08cd) and the P4 pre-step run on every P3 change of H01, H06, H09, H10, H11, H28 and H36, fc368e1 and 077d1fe included (base f8746a4; 3 findings, on H01, H06 and H09) | decisions-log M2-P3-B2-ROW2; M2-P4-B2-ROW2; Codex text outside the repo: scratchpad review/m2p3-doc.codex.txt, review/m2p4-b2.codex.txt |
| 3 Verifier | P4-ACCEPT-3 (wrote none of the rows it checks) | Checked at 0c4c70e. No Codex finding to sort. Rows written by others: the fc368e1 H11-Q2 fill. `Heap::BARRIER_THRESHOLD_OFFSET` = 0 (`crates/ep9_heap/src/heap.rs:330`); both `Heap` and `Hot` are `repr(C)`, and const asserts give `offset_of!(Heap, hot) == 0` and `offset_of!(Hot, barrier) + BarrierState::THRESHOLD_OFFSET == 0` (`crates/ep9_heap/src/heap.rs:213-214`); the threshold is the first field of `BarrierState` (`crates/ep9_heap/src/barrier.rs:80`); `Cx::rt` is `*const Runtime` (`crates/ep9_vm/src/interp/mod.rs:132`), and `object::wr` passes `heap.barrier()` to `Slot::set` (`crates/ep9_vm/src/object.rs:335-340`); `decided_constants` and `heap_field_offsets_hold_at_run_time` (which writes the byte through the barrier and reads it at heap offset 0) pass: true. Kind fill: agree (an as-built value for an Open question; no check changes). The 077d1fe ref in row 1 (`Heap::BARRIER_THRESHOLD_OFFSET` moved to `crates/ep9_heap/src/heap.rs:330`): true. Change-log kinds: agree | branch m0m2/p4-accept-3; local run, mac a64, 0c4c70e |
| 4 Data | from the "Validated by" line | `bash crates/ep9_heap/disasm-barrier.sh`: PASS on aarch64 (cell test 3 + header check 4 (`ldrb ldrb cmp b.ls`) + store). `cargo test -p ep9_heap --locked`: 84 pass, with `barrier_model_matches_the_decision` and `decided_form_passes_and_negative_controls_fail`. `cargo test -p ep9_unit --locked`: 28 pass. Missing: V1 (S1, M1); V2, V3, loom and TSan (S2b); the x86-64 fast path is the CI disasm job. P4 refresh (0c4c70e): `cargo test -p ep9_heap --locked` 87 pass (85 unit tests and 2 doctests), with the two model tests, `decided_constants` and `heap_field_offsets_hold_at_run_time`; `cargo test -p ep9_unit --locked` 28 pass. The real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3) | local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/disasm-barrier.log, heap.log, unit.log; P4: scratchpad m0m2/accept3/logs/heap.log, unit.log |
| Approval (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 approver: a second reviewer agent (README §3.2); it wrote none of the M3-P0 rows | **Not approved**: the review record is not complete. Checked at 70d79e5 (m0m3/p0-fix-docs). Content: true. The 2 refs are the only text change in this ADR besides the Change-log row (`git diff --word-diff 6b652e1..70d79e5`), and each new ref names the item its text names: `object::wr`, which passes `heap.barrier()` to `Slot::set`. ADRLINE: 0 FIX, 0 UNFIXABLE. Kind clarify: agree. Missing: rows 1-3, which the Change-log row names (README §3.1); none is written. No content change is needed. Approve again once the rows are written, or once the orchestrator logs that a ref-only move needs only one reviewer row (as the 2026-10-22 H28 approvals had) | local read, 70d79e5; scratchpad approve-p0/refpairs.py; ADRLINE (scratchpad p4docs/path_line_drift.py) |
| 1 Reviewer (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 approver (5c271b5; a second reviewer agent, wrote none of the M3-P0 rows) and the M3-P0 round-1 fixer reviewer (Claude, diff 6b652e1..70d79e5, wrote none of it); orchestrator decision M3-P0-ADRLINE-ACCEPTED names both as row 1 | Checked at 70d79e5. The approver compared each of the 2 moved refs with the code: both name `object::wr`, which passes `heap.barrier()` to `Slot::set`: true. Kind clarify: agree. The fixer reviewer (0 findings) spot-checked refs in H14, H18, H19, H20, H23, H25, H31, H34 and H03; its list does not name this ADR. | scratchpad m0m3/p0/fixer-result.json (keys "approve" and "review"); scratchpad approve-p0/refpairs.py; row written by the M3-P0 round-2 docs fixer from the reports named here |
| 2 Codex (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | orchestrator (Codex gpt-6.1-sol) | Round 1 (Codex ADR slice, base a04dd70; it ran before the ADRLINE run): no finding on H11. Round 2 (Codex ADR slice, base 6b652e1: the round-1 fixer docs diff, which holds this ADRLINE row): needs-attention, 2 findings: X-ADR1 (history rows whose refs the ADRLINE run moved: H36 and H25) and X-ADR2 (H20, the nullish receiver). | Codex text outside the repo: scratchpad m0m3/p0/codex-adr.out (round 1), m0m3/p0/codex2-adr.out (round 2); row written by the M3-P0 round-2 docs fixer from the reports named here |
| 3 Verifier (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 round-2 verifier (wrote neither the ADRLINE rows, nor the Codex text, nor this fix) | Checked at 5c271b5 with a word diff of every ADR file from 6b652e1. X-ADR1: real, (b), made by the round-1 ADRLINE --fix (d6c8620, eb2ad2c): 3 dated history rows got HEAD lines (H36 077d1fe row, H25 review row 3, H20 P0-VM3 step 2 row). X-ADR2 is on H20 (Draft): (c) for this record. No finding is on H11. Review row 3 names an item that the refreshed ref still points at (M2-P4-FIX-ADRLINE-ROWS). None of the 2 refs is a history ref. | verifier output (X-ADR1, X-ADR2, sorted; word diff of every ADR file 6b652e1 -> 5c271b5), held by the orchestrator for round 2; row written by the M3-P0 round-2 docs fixer from the reports named here |
| 4 Data (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 round-2 docs fixer (runs only) | ADRLINE (scratchpad p4docs/path_line_drift.py, with 5 history pins added in round 2) at 0bde51c: checked=360 ok=360 fixed=0 unfixable=0, exit 0 (log: scratchpad m0m3/p0/r2docs/drift3.txt); it is run again on the commit that adds these rows | local run, mac a64; logs outside the repo: scratchpad m0m3/p0/r2docs/ |
