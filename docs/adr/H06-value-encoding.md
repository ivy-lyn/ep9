# H06: Value encoding

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P4-ACCEPT; review record complete). Was: Draft (2026-10-20) |
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
| `Value`, `JsResult<Value>` and word0 are 8 B | Static asserts in `ep9_value` (`crates/ep9_value/src/lib.rs:44-55`: `Value`, `Option<Value>`, `JsResult<Value>` and `Word0` are 8 B); `cargo xtask m0-exit` item 8 checks that the 3 size asserts are there and runs `cargo check -p ep9_value --lib --target <host>`, which evaluates them (NOT-RUN with `--no-build-check`; `tools/xtask/src/m0_exit.rs:1965-2001`); its Windows column reads the CI build step, which compiles them too ([08 §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks) lists size asserts) | SP-Value, M0 wk2; M0 exit item 8 |
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
| H06-Q3 | The bit pattern of the canonical NaN | SP-Value | **Filled (M0 month 2 P3, M2-P3-OQ-H06-Q3).** As built: the canonical NaN is `PURE_NAN_BITS` = 0x7FF8_0000_0000_0000 (`crates/ep9_value/src/value.rs:39`, JSC PNaN). Boxing replaces every NaN with it (`Value::double`, `crates/ep9_value/src/value.rs:92-97`; `Value::number` boxes a NaN through it). Test `nan_payloads_are_purified` (`crates/ep9_value/src/tests.rs:208`): 11 NaN bit patterns (the x86 default NaN 0xFFF8_0000_0000_0000, signaling NaNs, patterns that would alias an int32, a cell or 0) and 6 NaNs made at run time all box to the same bits through `Value::double` and `Value::number`. H36 adds no constraint (decision H36-nan, 2026-10-20): it needs only purification on boxing; tests compare NaN as a class |

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
| 2026-10-22 | clarify | Rules row 1: its check named "xtask size asserts", which do not exist. It now names the `ep9_value` const asserts and the m0-exit item 8 check of them, as H08 Rules row 1 does (P2-ACCEPT-1, H08 R2). No check changes | P3-ACCEPT-2 |
| 2026-10-22 | fill | H06-Q3 filled as built: the canonical NaN is `PURE_NAN_BITS` 0x7FF8_0000_0000_0000 (JSC PNaN); test `nan_payloads_are_purified` proves the purification. Every check keeps its result (M2-P3-OQ-H06-Q3) | P3 integration fixer |
| 2026-10-22 | clarify | Rules row 1: m0-exit item 8 also compiles the asserts (`cargo check -p ep9_value` for the host target), so "no xtask task checks sizes" was false; the cell now says what item 8 runs (Codex C3, M2-P4-B2-ROW2). No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P4-ACCEPT). The §3.1 review record is complete: row 1 P3-ACCEPT-2, row 2 Codex (M2-P3-B2-ROW2 and the P4 pre-step run (M2-P4-B2-ROW2)), row 3 P4-ACCEPT-3 (and the P4 review checkers for the RF1 rows), row 4 data. The orchestrator did not write the rows it approves. No check changes | orchestrator |
| 2026-10-25 | clarify (proposed; Accepted path) | 1 `path:line` reference moved to the code after the M3-P3 merge and the M3-P3 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); each names the same item at the new line. The ref moved because GATES-2 changed `tools/xtask/src/m0_exit.rs` (the item 8 build check). The same ref in review row 3 (checked at 0c4c70e) keeps its line at that commit (a HISTORY pin of the drift script; M3-P1-REVIEWROW-REFS). Line refs moved by M3-P3; no meaning change. Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it, row 4 the ADRLINE output; the orchestrator approves | M3-P3 integration fixer |
| 2026-10-25 | clarify (proposed; Accepted path) | 1 `path:line` reference moved to the code after M3-P3 fix round 2 (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); it names the same item at the new line. The ref moved because the m0-exit gates fix (C-G1, C-G2) changed `tools/xtask/src/m0_exit.rs` (the item 8 build check moved down). Line refs moved; no meaning change. Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it, row 4 the ADRLINE output; the orchestrator approves | M3-P3 fix round 2 gates fixer |
| 2026-10-25 | clarify | Review record for the M3-P3 clarify rows (c3d7f70 and cbef98b): rows 1-4 (row 3 by the M3-P3 clarify row-3 verifier: each moved ref names the same line text at the commit that wrote the old ref and at the new commit) and two Approval rows: the cbef98b row Approved (Codex fix2-gates holds it); the c3d7f70 row Pending (no Codex run holds it; M3-P0-ADRLINE-ACCEPTED). No text outside the record changes; no check changes | M3-P3 records writer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P3-ACCEPT-2 (did not write the ADR) | Decision = the 02 §6 H6 row (the immediate bits are in the plan row since the 2026-10-20 fill). Checked against `ep9_value` at e719ffa: `encoding` (`NOT_CELL_MASK` = `NUMBER_TAG \| OTHER_TAG`, immediates 0x02, 0x06, 0x07, 0x0a, 0x22, `crates/ep9_value/src/value.rs:29-36`; `VERSION` = 1, carried in the unit header field `value_encoding`), NaN purification to `PURE_NAN_BITS` 0x7FF8_0000_0000_0000 on boxing (`value.rs:39`), the const asserts (`crates/ep9_value/src/lib.rs:44-55`), the tests `empty_avoids_jsc_reserved_patterns` and `immediates_have_the_documented_bits`, the `bits != 0` debug asserts of the heap slot visitor (`crates/ep9_heap/src/trace.rs:140`, :165) and the CI `miri` job. R1: Rules row 1 named "xtask size asserts", which do not exist (fixed, clarify, as H08 R2). Open and due in M0, not filled (reported): H06-Q3 (canonical NaN; the code already uses 0x7FF8_0000_0000_0000, JSC PNaN). Change-log kinds: agree | c68971a |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | Two runs. (1) M2-P3-B2-ROW2 ("ADR docs" slice of the P3 merge, base 66f08cd): no finding on H06. (2) The P4 pre-step run on every P3 change of H01, H06, H09, H10, H11, H28 and H36, fc368e1 included (base f8746a4): 3 findings. On H06, C3 (medium): Rules row 1 says no xtask task checks sizes, but m0-exit item 8 runs `cargo check -p ep9_value` for the selected target, which evaluates the const asserts | decisions-log M2-P3-B2-ROW2; M2-P4-B2-ROW2; Codex text outside the repo: scratchpad review/m2p3-doc.codex.txt, review/m2p4-b2.codex.txt |
| 3 Verifier | P4-ACCEPT-3 (wrote none of the rows it checks) | Checked at 0c4c70e. C3 reproduced: item 8 checks that the 3 `size_of` asserts are in `crates/ep9_value/src/lib.rs`, then runs `cargo check --locked -p ep9_value --lib --target <host>` and reports PASS only if it succeeds (`tools/xtask/src/m0_exit.rs:1270-1306`; NOT-RUN with `--no-build-check`). Real, category (b) (made by the P3-ACCEPT-2 Rules row 1 edit). Fixed (clarify row). Rows written by others: the P3-ACCEPT-2 Rules row 1 clarify (the const block is `crates/ep9_value/src/lib.rs:44-55`: true; its "no xtask size check" part is C3); the fc368e1 H06-Q3 fill (`PURE_NAN_BITS` = 0x7FF8_0000_0000_0000, `crates/ep9_value/src/value.rs:39`; `Value::double` replaces every NaN, `crates/ep9_value/src/value.rs:92-97`; `Value::number` boxes every non-int32 through `Value::double`; `nan_payloads_are_purified`, `crates/ep9_value/src/tests.rs:208`, has 11 bit patterns and 6 run-time NaNs and passes: true; kind fill: agree, no check changes result). Reported, not edited (H36 is Accepted): at 0c4c70e H36-Q5 (H36:121) still read "Owned by H06-Q3. H36 needs no particular pattern", though H06-Q3 is now filled. Since edited on the H36 Accepted path (H36 Change log, 2026-10-22, P4 review fixer (docs)): H36-Q5 now names `PURE_NAN_BITS`; the orchestrator approval and the amendment review record are in the H36 Review record (decision M2-P4-RF1-H36). Change-log kinds: agree | branch m0m2/p4-accept-3; local run, mac a64, 0c4c70e |
| 4 Data | from the "Validated by" line | SP-Value: the const asserts compile; `cargo test -p ep9_value --locked`: 20 pass; Miri (`cargo +nightly-2026-10-04 miri test --locked -p ep9_value`, as the CI job): 20 pass; `bash crates/ep9_heap/disasm-barrier.sh`: PASS on aarch64 (cell test 3 + header check 4 + store); SP-Value GO (decisions log W4-P4-SPIKE-RECHECK). Missing: the FP differential (H36, M1); the x86-64 barrier rows are the CI disasm job. P4 refresh (0c4c70e): `cargo test -p ep9_value --locked` 20 pass, `nan_payloads_are_purified` among them; the real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3) | local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/value.log, miri-value.log, disasm-barrier.log; P4: scratchpad m0m2/accept3/logs/value.log |
| 1 Reviewer (M3-P3 amendment: the 2026-10-25 clarify rows) | the ADRLINE runners (a mechanical run; they wrote these rows); the M3-P3 Claude reviewers, who wrote none of them (step 2: 8ef7da2..2178586; fix round 2: the 5 fix2 branches at 2178586); and, as a reviewer that wrote none of these rows, the M3-P3 records writer | Mechanical: the drift script (scratchpad p4docs/path_line_drift.py) maps each ref through the `git diff -U0` hunks from the commit that wrote it, and moves only the line number. Two moves of one ref, in the Rules row '`Value`, `JsResult<Value>` and word0 are 8 B' (the m0-exit item 8 build check): `m0_exit.rs` 1270-1306 -> 1844-1880 (c3d7f70, ADRLINE after the M3-P3 merge, M3-P3 integration fixer: GATES-2 changed `m0_exit.rs`) -> 1965-2001 (cbef98b, M3-P3 fix round 2 gates fixer: the C-G1 and C-G2 fix moved the check down). The same ref in review row 3 (checked at 0c4c70e) keeps 1270-1306 through a HISTORY pin (M3-P1-REVIEWROW-REFS). The Claude reviewers have no finding on these rows; each reports ADRLINE checked=413 ok=413 on its tree (2178586; the merged fix2 tree). The records writer compared each old line, at the commit that wrote the old ref, with the new line at the run commit: the same text (row 3). Kind clarify: agree (README §3.3: only line numbers in `path:line` refs change; no check, test or golden file changes) | step-2 review (8ef7da2..2178586): scratchpad m0m3/p3/step2-agents.json, the review entry; fix round 2 review (the 5 fix2 branches at 2178586): m0m3/p3/fix2-agents.json, the review entry; the ADRLINE commits named here |
| 2 Codex (M3-P3 amendment: the 2026-10-25 clarify rows) | orchestrator (Codex gpt-6.1-sol) | All runs: `/codex:adversarial-review --model gpt-6.1-sol`, in the background. cbef98b is in M3-P3 round 2, fix2-gates (base 2178586, head cbef98b): approve, no finding (Codex checked item 6 shard membership and the start/exit-status pairing; the six shards cover the 53,616 pinned files once); none on the moved ref or on this row. c3d7f70 is in no Codex diff: it is in workflow 2 (8ef7da2..2178586), which had the step-2 Claude review and no Codex run (round 1 ran on the step-1 role branches, base 88998a1; round 2 on the fix2 branches, base 2178586; round 3 on fix round 3, base b3020dd). Open finding: none. Missing: a Codex run that holds c3d7f70 | Codex texts outside the repo: scratchpad m0m3/p3/codex/r2-fix2-gates.out (and the other r1-*, r2-* and r3-*.out, checked for these rows), with run1.sh, run2.sh and run3.sh; row written by the M3-P3 records writer from those texts |
| 3 Verifier (M3-P3 amendment: the 2026-10-25 clarify rows) | M3-P3 clarify row-3 verifier (the M3-P3 records writer; it wrote no M3-P3 clarify row and no Codex text) | Checked at f5149b1. Each old line was read with `git show` at the commit that wrote the old ref, and each new line at the run commit; each ref at 88998a1 was also compared with its last form at f5149b1. `m0_exit.rs` 1270-1306 at 048a60e (the commit that wrote the ref), 1844-1880 at c3d7f70, and 1965-2001 at cbef98b and at f5149b1 are the same 37 lines: the item 8 build check, from `Ok(text) => {` (the `STATIC_ASSERTS` presence check) to the PASS cell that names `cargo check -p ep9_value --target <host>`. No other text outside the Change log changes in this ADR (`git diff --word-diff 88998a1 f5149b1`). No moved ref is in a dated review row; the review row 3 ref (checked at 0c4c70e) keeps 1270-1306 through the HISTORY pin, and line 1270 at 0c4c70e is `Ok(text) => {`. The Change-log counts are true. Kind clarify: agree (README §3.3). Codex (row 2): fix2-gates (holds cbef98b): approve, nothing to sort; c3d7f70: no Codex run, so nothing to sort, and row 2 is not complete for it. Rows 1, 2 and 4: true as written. Open (a)/(b): none | local read, f5149b1 (`git show` of each old and new line; `git diff -U0` of each commit named here); tools and logs outside the repo: scratchpad m0m3/p3/records/refcheck.py, refcheck.txt, basehead.sh, basehead.txt |
| 4 Data (M3-P3 amendment: the 2026-10-25 clarify rows) | the ADRLINE runners (runs only), and the M3-P3 records writer (the run at f5149b1) | ADRLINE dry runs: at c3d7f70 checked=413 ok=413 fixed=0 unfixable=0 (M3-P3 integration fixer); at 2178586 checked=413 ok=413 (step-2 Claude reviewer); at cbef98b checked=413 ok=413 fixed=0 unfixable=0 (fix round 2 gates fixer); at f5149b1 (before the records commits): checked=413 ok=413 fixed=0 unfixable=0, pinned=6, history=39 | local runs, mac a64; reports outside the repo: scratchpad m0m3/p3/step2-agents.json, m0m3/p3/fix2-agents.json; m0m3/p3/records/drift-pre.txt |
| Approval (M3-P3 amendment: the 2026-10-25 fix round 2 ADRLINE clarify row, cbef98b) | orchestrator (README §3.2; it wrote none of these rows and none of rows 1-4); row written by the M3-P3 records writer | **Approved.** Orchestrator approves (README §3.2, clarify on an Accepted ADR, review record complete; the row was written by the M3-P3 fix round 2 gates fixer, not the orchestrator). 2026-10-25. Rows 1-4 are present, row 2 has a Codex run that holds it, and row 3 finds no defect | orchestrator decision (M3-P3 notes: the clarify rows on the Accepted H06, H08, H10, H23, H24 and H28 get row 3 by a non-author agent, then the orchestrator approves); rows 1-4 above |
| Approval (M3-P3 amendment: the 2026-10-25 ADRLINE clarify row after the M3-P3 merge, c3d7f70) | orchestrator (README §3.2; it wrote none of these rows and none of rows 1-4); row written by the M3-P3 records writer | **Pending** (not approved). Rows 1, 3 and 4 are present, and row 3 finds no defect in the text. Row 2 is not complete: no Codex run holds c3d7f70 (it is in workflow 2 (8ef7da2..2178586), which had the step-2 Claude review and no Codex run (round 1 ran on the step-1 role branches, base 88998a1; round 2 on the fix2 branches, base 2178586; round 3 on fix round 3, base b3020dd)), and M3-P0-ADRLINE-ACCEPTED asks for the Codex ADR run also for a ref-only move on an Accepted ADR. Needed: a Codex run on a diff that holds c3d7f70 (for example `docs/adr` of 8ef7da2..2178586), a row-2 addendum and, for any finding, a row-3 sort; then the orchestrator approves |  |
