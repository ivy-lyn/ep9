# H07: Address space and code range

| Field | Value |
|---|---|
| Status | Draft (2026-10-20) |
| Fix by | M0 wk2 (SP-VA) |
| Freeze point | M1 exit |
| Origin | v1#3; plan status E (extended: code range) |
| Crates | `ep9_platform` (VA reserve incl. the code range), `ep9_heap` (cage, in-cage test) |
| Owner (plan role) | GC lead + JIT/platform lead; AOT senior for the code range |
| Validated by | SP-VA (M0 wk2) |
| Plan sources | [02 §6 H7](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [03 §5](../plan/03-gc.md#5-contract-items-that-exist-from-s0), [04 §5](../plan/04-execution.md#5-jit-infrastructure-and-the-relocatable-macroassembler), [08 §2](../plan/08-repo.md#2-crates) |

## Context

- H6 stores raw cell pointers in a 47-bit payload. So every heap address must be below 2^47 on every target.
- CSS (H10) needs a fast test: "is this word inside the heap?"
- JIT code and AOT units call each other and call stubs. On arm64, direct calls reach only 128 MiB.
- Goals: G6 (no abort on hostile input), G10 (4 tier-1 targets). Risk RK9: heap addresses above 47 bits (LA57, 52-bit arm64, sanitizers).

## Decision

One cage per process below 2^47 (64 GiB default, configurable); avoids 0x4001_0000_0000 + 32 GiB; startup self-check. Plus one code-range reservation for JIT code and mmapped units.

```
 0 .. first heap page ............................................. 2^47
 | immediates (H6) |   cage: all JS heap cells (64 GiB default)    |
                       never overlaps 0x4001_0000_0000 .. +32 GiB
                     + code range next to the cage: JIT code and mmapped units
```

- The code range sits next to the cage ([03 §5](../plan/03-gc.md#5-contract-items-that-exist-from-s0); SP-VA gate).
- `ep9_platform` owns the VA reserve, including the code range ([08 §2](../plan/08-repo.md#2-crates)).
- The plan fixes no addresses except the 2^47 bound for the cage and the avoided range. Where the code range sits is Open question H07-Q5.
- **macOS arm64 code range (`jit` feature; noted 2026-10-21).** `Cage::reserve_jit` maps the code range with `MAP_JIT` and read-write-execute protection at reservation. So this range has no inaccessible phase. Every other reservation, and the code range from `Cage::reserve`, starts inaccessible.
  - Why: macOS 27 rejects `MAP_JIT` with `MAP_FIXED` (the range is mapped at a hint address, then checked), and rejects every `mprotect` of a `MAP_JIT` page after it is RWX (SP-JIT-mem facts F1, F2: [data](../measurements/2026-10-21-sp-jit-mem.md)).
  - W^X still holds per thread: each thread sees the pages as RX or RW only (`pthread_jit_write_protect_np`). `Reservation::commit` and `decommit` refuse this range; `ep9_platform::jit::CodeMemory` manages it, and release uses `madvise(MADV_FREE_REUSABLE)`.
  - Code: `crates/ep9_platform/src/va.rs` (`Reservation::reserve_code_range_at`), `src/sys/unix.rs` (`reserve_map_jit_at`), `src/jit/`.

## Rules

| Rule | Check | From |
|---|---|---|
| Every cage address is below 2^47 | Startup self-check: if the reservation fails, ep9 refuses to run (RK9) | M0 wk2 |
| The cage never overlaps 0x4001_0000_0000 + 32 GiB | Startup self-check; SP-VA on 4 targets | M0 wk2 |
| One cage per process; isolates share it | Isolate-per-thread model (H22, D14) | M0 wk3 |
| JIT code and mmapped units live in the code range | SP-VA; later SP-JIT-mem and SP-Unit place code there | M0 wk2-month 2 |
| CSS rejects any word outside the cage with one range check | Cage range check before the VO lookup (03 §2) | SP-GC |

## Consequences

- **Gains.** The payload always fits. The in-cage test for CSS is fast. The design is safe on LA57 and 52-bit VA hosts.
  - Direct calls between JIT code, units and stubs stay possible inside one reservation. V8 copied builtins into its cage for this reason [R11].
- **Costs.** A fixed reservation per process. 64 GiB is the default; embedders can configure it.
  - Hosts with small VA limits must lower the size. The startup self-check makes a failure visible instead of a later crash.
- **Follow-ups.**
  - D5 (WebAssembly): "Signal and VA coexistence rules apply now" (11-decisions). A later wasmtime engine must fit beside the cage and code range.
  - H35: `.ep9u` blobs are mapped into the code range; `.o` units are linked into the binary.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R09] | Value-repr resolution: reserve the whole heap below 2^47. Avoid the fixed tsrs range at 0x4001_0000_0000 (32 GiB) | [09-critic.md](../research/09-critic.md) |
| [R07] | Boa #4275: raw malloc pointers above 48 bits panic on Android aarch64. Lesson: own reserved region | [07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R06] | LA57 and 52-bit VA break NaN-boxing unless address-space reservation is controlled | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R11] | arm64 direct calls reach only 128 MiB; Apple M1 never predicts targets beyond 4 GiB | [11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |

## Validation

- **SP-VA** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk2, GC lead + JIT/platform lead): cage below 2^47 on 4 targets, plus the code range next to it. On fail it reopens H7.
- Week 2 check: "SP-VA reserves cage and code range on 4 targets" ([09 §8](../plan/09-milestones.md#8-m0-first-month-week-by-week)).
- SP-JIT-mem (wk3) and SP-Unit (month 2) put code in the code range.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H07-Q1 | Size of the code range. The plan states the 128 MiB direct-call reach but no size | SP-VA | Measure in SP-VA; record as a fill |
| H07-Q2 | What happens to calls beyond direct-call reach (indirect call or veneer) | SP-JIT-mem / H34 | Decide in the H34 ADR (relocation kinds) |
| H07-Q3 | Is 0x4001_0000_0000 + 32 GiB still needed? tsrs is never in the engine process (N8). The plan keeps the rule | M0 wk2 | Keep the rule; it costs nothing |
| H07-Q4 | Windows: reserve with VirtualAlloc at a hinted address below 2^47; behavior if the hint fails | SP-VA | Retry, then refuse to run (self-check) |
| H07-Q5 | Must the code range also be below 2^47? H6 requires it only if a code address is ever boxed as a Value. The plan does not say. "Next to the cage" is the only rule | SP-VA | Decide in SP-VA |

## Reopens if

- SP-VA fails on any tier-1 target (09 §4 names H7).
- The startup self-check fails on a tier-1 CI runner.
- D5 changes to "WebAssembly in v1" and the coexistence rules do not fit (11-decisions "Affects": H7, H28).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6, 03 §5, 04 §5 (M0 wk1 list) | ADR agent |
| 2026-10-21 | clarify | Decision notes that on macOS arm64 the code range from `Cage::reserve_jit` is RWX `MAP_JIT` from the start (no inaccessible phase), as built for SP-JIT-mem | docs-sync agent |
