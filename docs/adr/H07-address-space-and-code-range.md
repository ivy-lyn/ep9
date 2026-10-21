# H07: Address space and code range

| Field | Value |
|---|---|
| Status | Draft (2026-10-20; rewritten 2026-10-21) |
| Fix by | M0 wk2 (SP-VA); the process code range before Spike B (M0 month 2) |
| Freeze point | M1 exit |
| Origin | v1#3; plan status E (extended: code range) |
| Crates | `ep9_platform` (VA reserve incl. the code range), `ep9_heap` (one cage per heap, in-cage test) |
| Owner (plan role) | GC lead + JIT/platform lead; AOT senior for the code range |
| Validated by | SP-VA (M0 wk2; passed on 4 CI runners); Spike B and SP-Unit put the first code in the process code range |
| Plan sources | [02 §6 H7](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [03 §5](../plan/03-gc.md#5-contract-items-that-exist-from-s0), [04 §5](../plan/04-execution.md#5-jit-infrastructure-and-the-relocatable-macroassembler), [08 §2](../plan/08-repo.md#2-crates), [H22](H22-isolates-and-realms.md) |

## Context

- H6 stores raw cell pointers in a 47-bit payload. So every heap address must be below 2^47 on every target.
- CSS (H10) needs a fast test: "is this word inside the heap?"
- JIT code and AOT units call each other and call stubs. On arm64, direct calls reach only 128 MiB.
- D14: one isolate per thread, each with its own heap (H22). A heap that only its own thread uses needs no shared allocator.
- Goals: G6 (no abort on hostile input), G10 (4 tier-1 targets). Risk RK9: heap addresses above 47 bits (LA57, 52-bit arm64, sanitizers).

## Decision

One cage per isolate below 2^47 (64 GiB default, configurable); avoids 0x4001_0000_0000 + 32 GiB; startup self-check. Plus one process-wide code range for JIT code and mmapped units, built before Spike B (decision W4-H07-REWRITE; built in M0 month 2 P1, H07-Q6).

```text
 0 .. first heap page ......................................................... 2^47
 | immediates (H6) | cage A (isolate A) | ... | cage B (isolate B) | ... | code range |
                     64 GiB each, 4 GiB-aligned slots, never over 0x4001_0000_0000 + 32 GiB
                                        one code range per process: JIT code + unit text (RX)
```

- **Cage per isolate (as built).** `Heap::new` reserves its own `Cage` (`crates/ep9_heap/src/memory.rs:154`). The in-cage test and the conservative scan look only at that heap's chunks (`crates/ep9_heap/src/memory.rs:250-252`, `crates/ep9_heap/src/trace.rs:357-358`). So one isolate's scan never finds another isolate's cells.
- **Process code range (as built, M0 month 2 P1).** Unit text is shared per process (05 §8, H22), and JIT stubs are shared by every isolate. One range keeps direct calls between them. SpiderMonkey also keeps one executable region per process ([SM-PEM]).
  - `CodeRange::process()` (`crates/ep9_platform/src/va.rs:619`) reserves it at the first call, on any thread, and returns the same `&'static CodeRange` after that. Placement and size: H07-Q6.
  - Users take page-granular `CodeChunk`s from it (`CodeRange::alloc`, `crates/ep9_platform/src/va.rs:693`: lowest free run that fits, behind a mutex; drop gives the pages back and merges free runs). With `jit`, `CodeChunk::commit` makes a chunk a W^X `jit::CodeMemory`.
- **No per-cage code range.** A cage reserves no code range by default (`CageConfig::default`, `code_range_size: 0`), so `Heap::new` reserves none (test `heap_new_reserves_no_code_range`). A non-zero `code_range_size` still gives a plain reservation right after the cage (`Cage::code_range`); no code goes there. `Cage::reserve_jit` and `Cage::code` are removed (month 2 P1 integration fixer): their last users, the unwind self-test and the `spike-native` stub tests, take `CodeRange::process()` chunks.
- `ep9_platform` owns the VA reserve, including the code range ([08 §2](../plan/08-repo.md#2-crates)).
- **macOS arm64 code range (`jit` feature; noted 2026-10-21).** In a `jit` build the process code range is mapped with `MAP_JIT` and read-write-execute protection at reservation. So this range has no inaccessible phase. Every other reservation, a code range in a build without `jit`, and the per-cage range from `Cage::reserve`, start inaccessible.
  - Why: macOS 27 rejects `MAP_JIT` with `MAP_FIXED` (the range is mapped at a hint address, then checked), and rejects every `mprotect` of a `MAP_JIT` page after it is RWX (SP-JIT-mem facts F1, F2: [data](../measurements/2026-10-21-sp-jit-mem.md)).
  - W^X still holds per thread: each thread sees the pages as RX or RW only (`pthread_jit_write_protect_np`). `Reservation::commit` and `decommit` refuse this range; `ep9_platform::jit::CodeMemory` manages it, and release uses `madvise(MADV_FREE_REUSABLE)`.
  - Code: `crates/ep9_platform/src/va.rs` (`Reservation::reserve_code_range_at`), `src/sys/unix.rs` (`reserve_map_jit_at`), `src/jit/`. The process code range keeps this mapping rule.

## Rules

| Rule | Check | From |
|---|---|---|
| Every cage and code-range address is below 2^47 | `Cage::self_check` at reserve: if it fails, the isolate is not created (RK9); `CodeRange::self_check` at reserve: if it fails, no code range exists; SP-VA on 4 targets | M0 wk2 |
| No cage or code range overlaps 0x4001_0000_0000 + 32 GiB | `Cage::self_check`, `CodeRange::self_check`; `sp_va_avoids_tsrs_range`, `sp_va_code_range_avoids_tsrs_range` | M0 wk2 |
| Each isolate owns one cage; a cell lives only in its own isolate's cage | `Heap::new` reserves the cage; `find_cell` reads only its own chunks | M0 wk2 |
| CSS rejects any word outside its cage with one range check | `Memory::contains` before the VO lookup (03 §2) | SP-GC |
| JIT code and mmapped units live in the one process code range | `CodeRange::process` gives every thread the same range (unit test `process_code_range_is_one_per_process`); Spike B and SP-Unit place code there | before Spike B |
| The process code range is 128 MiB on every target and never larger than the direct-branch reach (128 MiB on arm64, 2 GiB on x86-64); a per-cage range (month 2 P1 only) obeys the same limit | `CodeRangeConfig::validate`, `CodeRange::self_check`; `CageConfig::validate`, `Cage::self_check` | M0 month 2 P1 |

## Consequences

- **Gains.** The payload always fits. The in-cage test for CSS is fast and also rejects cells of other isolates. The design is safe on LA57 and 52-bit VA hosts.
  - Direct calls between JIT code, units and stubs stay possible inside the one code range. V8 copied builtins into its pointer-compression cage for this reason [R11].
- **Costs.** 64 GiB of address space per isolate by default, plus 128 MiB once per process for the code range. At most 2,046 default cages fit below 2^47 (16 slots of 4 GiB each; the tsrs range blocks one cage), fewer when the code range or other mappings take slots. Embedders and workers can lower the size (H22-Q8).
  - Hosts with small VA limits must lower the size. The startup self-check makes a failure visible instead of a later crash.
- **Follow-ups.**
  - Done in M0 month 2 P1: the process code range is built, the per-cage code range is off by default, and `Cage::reserve_jit` and `Cage::code` are removed (their last users moved to `CodeRange::process()`).
  - D5 (WebAssembly): "Signal and VA coexistence rules apply now" (11-decisions). A later wasmtime engine must fit beside the cages and the code range.
  - H35: `.ep9u` blobs are mapped into the code range; `.o` units are linked into the binary.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R09] | Value-repr resolution: reserve the whole heap below 2^47. Avoid the fixed tsrs range at 0x4001_0000_0000 (32 GiB) | [09-critic.md](../research/09-critic.md) |
| [R07] | Boa #4275: raw malloc pointers above 48 bits panic on Android aarch64. Lesson: own reserved region | [07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R06] | LA57 and 52-bit VA break NaN-boxing unless address-space reservation is controlled | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R11] | arm64 direct calls reach only 128 MiB; Apple M1 never predicts targets beyond 4 GiB | [11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [SM-PEM] | SpiderMonkey: one per-process executable reservation; near jumps between JIT pages; the Win64 exception handler is registered once | mozilla-firefox `js/src/jit/ProcessExecutableMemory.cpp` lines 665-679 (main, read 2026-10-21) |

## Validation

- **SP-VA** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk2, GC lead + JIT/platform lead): cage below 2^47 on 4 targets, plus the code range. On fail it reopens H7.
  - **Result: PASS** on macOS arm64, Linux x64, Linux arm64 and Windows x64: `crates/ep9_platform/tests/sp_va.rs` (8 tests; 9 on Linux) in build-test, CI run 39518270586 on the week-4 base tree.
- SP-JIT-mem (wk3) put code in a cage's code range (`Cage::reserve_jit`, removed in month 2 P1); since month 2 P1 it puts code in a chunk of the process code range. Spike B and SP-Unit (month 2) put code in the process code range.
- Month 2 P1 unit tests (`crates/ep9_platform/src/jit/tests.rs`): two threads get the same range; two cages without a code range, on two threads, each run code in their own chunk at the same time; code in a chunk is callable, the system unwinder walks through it and a panic unwinds through it; the range is below 2^47 and outside the tsrs range.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H07-Q1 | Size of the code range | SP-VA | **Filled 2026-10-21 (as built): 128 MiB default**, the arm64 direct-call reach; a size above the target's direct-branch reach is refused (`crates/ep9_platform/src/va.rs:587-588`) |
| H07-Q2 | What happens to calls beyond direct-call reach (indirect call or veneer) | SP-JIT-mem / H34 | Decide in the H34 ADR (relocation kinds) |
| H07-Q3 | Is 0x4001_0000_0000 + 32 GiB still needed? tsrs is never in the engine process (N8) | M0 wk2 | **Filled 2026-10-21 (as built): kept**; `TSRS_RESERVATION` is in the default avoid list |
| H07-Q4 | Windows: reserve at a hinted address below 2^47; behavior if the hint fails | SP-VA | **Filled 2026-10-21 (as built):** `VirtualAlloc` at each slot address; a used slot is skipped (`sp_va_skips_used_slot`); no free slot gives `CageError::NoFreeSlot`, and the isolate is not created |
| H07-Q5 | Must the code range be below 2^47? | SP-VA | **Filled 2026-10-21 (as built): yes** (`Cage::self_check`). The process code range keeps it: frames store return pcs as native words (addr + 2^49, decision T0-NATIVE-WORD) |
| H07-Q6 | Where does the process code range sit, how big is it, and when is it reserved? | Spike B | **Filled 2026-10-21 (as built, M0 month 2 P1):** below 2^47 and outside the tsrs range, found by the cage slot search (`CodeRangeConfig::default`: 4 GiB-aligned slots from 4 GiB, a seeded random first slot, each slot tried at its exact address and skipped if not free); 128 MiB on every target (`DEFAULT_CODE_RANGE_SIZE`); reserved at the first `CodeRange::process()` call, once per process and never released (a failed call reserves nothing; the next call tries again); on macOS arm64 with `jit` one `MAP_JIT` RWX mapping placed by address hint and then checked (no `MAP_FIXED`). Code takes `CodeChunk`s of whole pages from it |

## Reopens if

- SP-VA fails on any tier-1 target (09 §4 names H7).
- The startup self-check fails on a tier-1 CI runner.
- Spike B or SP-Unit cannot place code in one process code range on a tier-1 target.
- D5 changes to "WebAssembly in v1" and the coexistence rules do not fit (11-decisions "Affects": H7, H28).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6, 03 §5, 04 §5 (M0 wk1 list) | ADR agent |
| 2026-10-21 | clarify | Decision notes that on macOS arm64 the code range from `Cage::reserve_jit` is RWX `MAP_JIT` from the start (no inaccessible phase), as built for SP-JIT-mem | docs-sync agent |
| 2026-10-21 | rewrite | Decision and Rules follow the code: one cage per isolate (as built since M0 wk2) and one process-wide code range, built before Spike B. Old Decision: "One cage per process below 2^47 (64 GiB default, configurable); avoids 0x4001_0000_0000 + 32 GiB; startup self-check. Plus one code-range reservation for JIT code and mmapped units". Old Rules row: "One cage per process; isolates share it", checked by "Isolate-per-thread model (H22, D14)". Also: H07-Q1, Q3, Q4 and Q5 filled from SP-VA and the code; H07-Q6 added. Decision W4-H07-REWRITE (decisions log §11); the plan row 02 §6 H7 changes in P4 | P0-DOCS agent |
| 2026-10-21 | clarify | 3 `path:line` references moved to the code at `479b1cd` (P4-DOCS drift script, [I15]) | P4-DOCS agent |
| 2026-10-21 | fill + rewrite | Fill: H07-Q6 (placement, size, reservation time of the process code range, as built by `CodeRange::process`). Rewrite: the per-cage code range is off by default (`CageConfig::default` has `code_range_size: 0`; 0 was refused before), so the Decision bullet "Until it exists. Each cage reserves a code range right after itself (the old `DEFAULT_CODE_RANGE_SIZE` default in `crates/ep9_platform/src/va.rs`, `Cage::self_check`). The engine does not use it yet: no JIT code and no unit is loaded. It costs address space only." is replaced by "No per-cage code range"; the bullet "Process code range (to build)" becomes "as built"; the Rules row "A code range is at most the direct-branch reach (128 MiB on arm64, 2 GiB on x86-64)", checked by "`CageConfig::validate`, `Cage::self_check`", now names the process range and its checks; the first two Rules rows also name `CodeRange::self_check`; Costs: 64 GiB per isolate plus 128 MiB per process (was 64 GiB + 128 MiB per isolate, 1,926 cages); H07-Q1 ref moved to the code-range check | P1-CODE-RANGE agent |
| 2026-10-21 | clarify | `Cage::reserve_jit` and `Cage::code` removed after their last users (unwind self-test, `spike-native` stub tests) moved to `CodeRange::process()` chunks; the "No per-cage code range" bullet, the macOS arm64 bullet and the Follow-ups line say so. A non-zero `code_range_size` stays as a plain reservation | M2-P1 integration fixer |
