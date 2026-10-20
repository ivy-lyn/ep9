# H06: Value encoding

| Field | Value |
|---|---|
| Status | Draft (2026-10-20) |
| Fix by | M0 wk2 |
| Freeze point | M1 exit |
| Origin | v1#2; plan status U (unchanged) |
| Crates | `ep9_value` (defines `Value`); every tier and code kind that boxes or tests values |
| Owner (plan role) | VM lead (SP-Value owner) |
| Validated by | SP-Value (M0 wk2) |
| Plan sources | [02 §6 H6](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [06 §1](../plan/06-object-model-and-spec.md#1-values-h6-unchanged), [03 §2](../plan/03-gc.md#2-roots) |

## Context

- Every tier, builtin, IC, unit and the GC read and write values. The encoding is the most-copied contract after the builtin ABI.
- The encoding sets what the conservative stack scan (H10) can see and what the barrier (H11) tests.
- Goals: G2-G4 (speed), G7 (bounded memory cost). Non-goal N6: no V8-style pointer compression in v1.
- Depends on H7: cell pointers must be below 2^47, inside the cage.

## Decision

JSC NaN-box: raw cell pointers, int32 under the 0xFFFE tag, doubles + 2^49. `Value(NonZeroU64)`. Empty/hole is the non-zero immediate 0x22 (decision H06-empty, 2026-10-20). NaN purified on boxing.

| Kind | 64-bit pattern | Test |
|---|---|---|
| Cell pointer | `0x0000_PPPP_PPPP_PPPP`. P < 2^47, 8-byte aligned, inside the cage, never 0 | `(v & NotCellMask) == 0`, NotCellMask = NumberTag \| OtherTag = `0xFFFE_0000_0000_0002` (H06-Q2) |
| int32 | `0xFFFE_0000_IIII_IIII` | `v >= NumberTag` |
| double | raw f64 bits + 2^49. Top 16 bits run 0x0002-0xFFFC | number and not int32 |
| Immediates | null 0x02, false 0x06, true 0x07, undefined 0x0a, empty/hole 0x22. Each has the "other" tag bit 0x2 set and is below the first heap page (H06-Q1) | exact compare; misc `v <= 0x0E` excludes empty |

- `#[repr(transparent)] struct Value(NonZeroU64)`. So `JsResult<Value>` is 8 bytes and returns in one register.
- **NaN purification.** Every NaN becomes the one canonical NaN when it is boxed. This covers Float64Array reads, DataView reads and Math results.
- **No boxed doubles.** Doubles never allocate.
- **Empty/hole = 0x22, not 0.** 0 is the niche that keeps `Option<Value>` (None) and `JsResult<Value>` (Err) at 8 bytes. 0x22 fails every other kind test (cell, number, bool, undefined-or-null), is above JSC's MiscTag 0x0E, and is not JSC's BigInt32 tag 0x12, NativeCallee tag 0x3 or deleted value 0x4.
- **No 0 in a heap slot.** A heap slot typed `Value` never holds 0. Every Value slot of a new object is pre-filled (undefined, or 0x22 for a new holey store) before the next GC point. `is_cell(0)` is true, so the release slot visitor skips 0 and never dereferences it; debug builds assert `bits != 0`.
- **Encoding version.** The unit header records the value-encoding version (H35). On a mismatch the loader ignores the unit's native code and runs its bytecode.

## Rules

| Rule | Check | From |
|---|---|---|
| `Value`, `JsResult<Value>` and word0 are 8 B | Static asserts in `ep9_value`; xtask size asserts ([08 §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks)) | SP-Value, M0 wk2; M0 exit item 8 |
| No NaN other than the canonical one is ever boxed | Unit tests on every boxing path; FP differential (H36) | M0; M1 |
| A boxed cell pointer is the raw address | CSS probe case S6 does not apply (03 §2) | SP-GC |
| The encoding version is a unit header field | H35 header struct + golden test | M0 exit |
| Empty/hole avoids the JSC reserved patterns (0x04, BigInt32 `(i<<16)\|0x12`, NativeCallee `&7 == 3`) and fails every other kind test | Test `empty_avoids_jsc_reserved_patterns` in `ep9_value` | M0 wk2 |
| Every Value slot is pre-filled before the next GC point; the visitor skips 0 | Debug assert in the heap slot visitor; GC-stress | S0 |

## Consequences

- **Gains.** No HeapNumber and no pointer decode. CSS sees raw pointers. `JsResult<Value>` is 8 B in one register.
  - Encoded doubles start at 2^49, so they never fall in the cage. CSS cannot confuse a double with a cell (03 §2, case S6).
  - V8 still pays for boxed doubles: its mutable-heap-number work gave up to 2.5x on async-fs [R06].
- **Costs.** Slots are 8 bytes, as in node 24, which has no pointer compression. V8 with compression saves up to 43% of heap [R06].
  - G7 caps both ratios: ≤ 1.2x node --jitless (est.) and ≤ 1.8x a compressed d8 --jitless. A failed cap at M2 moves S3 into M5; it does not reopen H6 (02 G7).
- **Follow-ups.** H7 must keep every heap address below 2^47 on all 4 targets. H36 owns NaN purification of Math results.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | JSC encoding: raw pointers, doubles + 2^49, int32 under the FFFE tag (JSCJSValue.h). V8 compression saves up to 43% of heap | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R05] | Probe point 4: NaN-boxed or high-tagged pointers are invisible to CSS unless untagged | [05-gc-design.md](../research/05-gc-design.md) |
| [R09] | Value-repr resolution: raw-pointer NaN-boxing in a region below 2^47 | [09-critic.md](../research/09-critic.md) |
| [R07] | Boa issue #4275: NaN-boxing of raw malloc pointers panics above 48 bits | [07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [P1] | Plan v1 probe: 8 vs 16 B for `JsResult<Value>`. SP-Value checks it again | Session-local |

## Validation

- **SP-Value** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk2, VM lead): `ep9_value` static asserts Value, JsResult, word0 = 8 B. Barrier fast path ≤ 5 instructions on aarch64 plus the cell test. On fail it reopens H6 and H11.
- M0 exit item 8: static asserts Value, JsResult and word0 are 8 B.
- Miri runs on `ep9_value` ([10-quality §5](../plan/10-quality.md#5-other-ci-checks)): on every PR until the nightly trigger lands (CI job `miri`, since 2026-10-21), then nightly.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H06-Q1 | The exact bit values of null, undefined, false, true and empty/hole. JSC uses 0 for its empty value, but `NonZeroU64` forbids 0. The plan fixes only "small non-zero constants below the first heap page" | SP-Value, M0 wk2 | **Resolved (fill, 2026-10-20).** As JSC, except empty: null 0x02, false 0x06, true 0x07, undefined 0x0a, empty/hole 0x22 (OtherTag 0x2 \| EmptyTag 0x20; JSC uses 0x10 for BigInt32). Source: `ep9_value::encoding` (`VALUE_*`); test `immediates_have_the_documented_bits`. Confirmed by decision H06-empty (2026-10-20). 02 §6 H6 and 06 §1 now state the bits |
| H06-Q2 | The value of `NotCellMask`. Immediates must have a bit in it, or the cell test accepts them | SP-Value | **Resolved (fill, 2026-10-20).** As JSC: NumberTag \| OtherTag = `0xFFFE_0000_0000_0002`. Every immediate has OtherTag set. Source: `ep9_value::encoding::NOT_CELL_MASK`; same test. Confirmed by decision H06-empty (2026-10-20). 02 §6 H6 and 06 §1 now state the bits |
| H06-Q3 | The bit pattern of the canonical NaN | SP-Value | Open. H36 adds no constraint (decision H36-nan, 2026-10-20): it needs only purification on boxing; tests compare NaN as a class |

## Reopens if

- SP-Value fails its gate (09 §4 names H6 and H11).
- H7 reopens and the cage can no longer keep cell pointers below 2^47 on a tier-1 target.
- No D decision lists H6 in its "Affects" column ([11-decisions §3](../plan/11-decisions.md#3-decisions-all-decided-2026-10-20)). A G7 miss at M2 moves S3 into M5; it does not reopen H6.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft, copied from 02 §6 and 06 §1 (M0 wk1 list) | ADR agent |
| 2026-10-20 | fill | H06-Q1 and H06-Q2 resolved from `ep9_value::encoding` (SP-Value): immediate bit values and NotCellMask added to the Decision table. The plan row (02 §6 H6, 06 §1) still says "small non-zero constants"; plan fix to be filed with the plan owner | fix agent |
| 2026-10-20 | fill | H06-empty: 0x22 kept after a check against the JSC tags (BigInt32 0x12, NativeCallee 0x3, deleted 0x4, MiscTag 0x0E) and every ep9_value fast test. Rule added: no 0 in a heap Value slot (pre-fill before the next GC point; the visitor skips 0). Plan rows 02 §6 H6 and 06 §1 updated | apply agent |
| 2026-10-21 | clarify | Validation: Miri runs on every PR until the nightly trigger lands, as CI does ([decisions-log §10](../decisions-log.md#10-2026-10-21-pr-1-review-fixes-ci-gates-and-h15-on-t0) CIG-5) | follow-up agent |
