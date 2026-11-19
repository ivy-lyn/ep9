# H37: TS hint side-table schema

| Field | Value |
|---|---|
| Status | Draft (2026-11-18): schema v0, written as a design by M9-P4 A1H-TEXT (docs only). Nothing is built: no `ep9_hints` crate, no `tools/ep9-typecheck`, no join in `ep9_frontend`. The A1h build does not start in month 9 (M9-PLAN-D17) |
| Fix by | v0 at A1h (by M4 exit); frozen at A4 start |
| Freeze point | A4 start (README §2; README §3.4 row H37: "Schema v0 (A1h)"). One part (the Freeze table below) |
| Origin | R16-D13 + R17; plan status N (new) |
| Crates | `ep9_hints` (the `.ep9h` reader and writer; planned, A1h; not in `crates/` yet), `ep9_frontend` (the hint join), `ep9_facts` (the HINTED FactSnapshot), `tools/ep9-typecheck` (own workspace; planned, A1h) |
| Owner (plan role) | AOT lane: the AOT senior (author at A1h; 09 §4 SP-HintPrecision). This v0: M9-P4 A1H-TEXT (AOT lane, docs only) |
| Validated by | The A1h gate: the join matches ≥ 99.5% of core node kinds (est.), and the SP-HintPrecision report exists; SP-RegionCoverage; SP-tsrs-win again at A1h; the dependency guards of 08 §7 |
| Plan sources | [02 §6 H37](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [05 §6](../plan/05-aot.md#6-ep9-typecheck-the-tsrs-type-oracle), [05 §7](../plan/05-aot.md#7-side-table-schema-h37-crate-ep9_hints), [01 §10](../plan/01-front-end.md#10-hint-join-front-end-side), [01 §11](../plan/01-front-end.md#11-ep9-typecheck-summary), [08 §5](../plan/08-repo.md#5-how-third_partytsrs-is-consumed), [08 §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks), [09 §3](../plan/09-milestones.md#3-aot-stages-a0-a5), [doc 13 §4](../plan/13-execution-modes.md#4-facts-and-profiles-across-runs) |

## Context

- D18 is (b): a cheap type oracle now (A1h), hint consumers only in A4, and a kill by K-A4H ([11-decisions](../plan/11-decisions.md)). The oracle is tsrs, in the separate binary `ep9-typecheck`. It is never linked into the engine (N8, H26).
- The schema is ep9-owned so that tsrs churn stays outside the engine: the oracle built against a 15-day-old tsrs with 6 compile errors [R13]. The schema also absorbs the BOM, UTF-16 and lossy-UTF-8 offset shifts of the tsrs decoder [R13], and the Unicode pin (below).
- A hint is never a fact. HINTED is the lowest source (H30: RUNTIME > PROFILED > HINTED, per fact kind); TS types never become PROVEN, and every HINTED fact needs a guard or a fuse (H30 Rules). 24 crafted hole classes pass strict checks and lie at run time [R12]; the tsrs hint is wrong in each of 13 crafted oracle holes [R13].
- Depends on: H1 (the join is inside `ep9_frontend`), H2 (the stable function id; the CompiledUnit hint section stays empty), H3 (the three TS flags), H16 and H30 (the FactSnapshot schema and the HINTED source), H32 (the guard exit of a failed HINTED guard), H35 (the container `facts` section), H26 (the tsrs workspace and the lock guard). Depended on by: the A4 hint consumers (K-A4H), SP-HintPrecision, SP-RegionCoverage (K-A5v-cov), the L2 class records (doc 13 §4.1).

## Decision

Copied from the plan row (02 §6): an ep9-owned binary schema in `ep9_hints` (zero dependencies). Key (start, end, kind) + decoded-text hash. dts/lib flags. Header: tsrs commit, fact-schema version, TS flags. About 20 B per node; no type strings. Files with type errors dropped. Join on the pre-lowering oxc AST. Converted to HINTED entries keyed by (function id, feedback slot).

```text
 .ep9h (05 §7)
 +- header      magic | schema version | tsrs commit | fact-schema version | TS flags
 |              (useDefineForClassFields, experimentalDecorators, emitDecoratorMetadata)
 +- file table  path | decoded-text hash | has_type_errors | record range
 +- classes     class id -> decl file:offset, field order, slot count
 +- records     per file, sorted by (start, end, kind), about 20 B each:
 |              repr category | class id | slot candidate | call-target decl | non-null |
 |              enum const | element type | dts/lib | confidence
 +- strings     paths only; no type strings
```

| # | Schema v0 rule (from the plan) |
|---|---|
| S1 | The key of a record is the oxc span (start, end) plus the node kind; the tsrs side writes (skip_trivia(pos), end) (01 §10) |
| S2 | The file check is a hash of the decoded text. ep9 strips a UTF-8 BOM before offsets and hashing, as tsrs does (01 §10) |
| S3 | A hash or kind mismatch drops every hint of that file; `has_type_errors` = 1 drops them too. ep9c counts and reports each drop (01 §10, 05 §7) |
| S4 | A JSDoc cast keys on the ParenthesizedExpression span, because the cast shares its span with the inner node (01 §10) |
| S5 | A header mismatch (schema version, fact-schema version or TS flags) means ep9 ignores the file (05 §7; the same three flags as the unit header, H03) |
| S6 | No type strings: the strings table holds paths only (05 §7) |
| S7 | The joined records become a HINTED FactSnapshot (`ep9_facts` format, source `Hinted` = 2, `crates/ep9_facts/src/bits.rs:142-143`), keyed by (function id, feedback slot); the function id is the preorder index of H2 (01 §5) |
| S8 | That snapshot travels beside the CompiledUnit, in the unit `facts` section (container section kind 6, `crates/ep9_unit/src/codes.rs:248-249`; H35). The CompiledUnit hint section stays empty (doc 13 F-R1; `docs/adr/H02-bytecode-abi.md:43`) |
| S9 | Hints are optional in every mode. JIT mode needs none; plain JS uses PROVEN and PROFILED facts only. No codegen reads hints before A4 (`docs/plan/01-front-end.md:238`) |

### Freeze table (README §3.4)

| Part | What it holds | Freeze point | After the freeze point | Check |
|---|---|---|---|---|
| Schema v0 | The `.ep9h` layout (header, file table, classes, records, strings), the key, S1-S9 and the Q1-Q6 fills | A4 start (README §3.4 row H37) | Frozen-part rules (README §3.2) | the `ep9_hints` round-trip tests and the A1h gate (Validated by) |

## Rules

| Rule | Check | From |
|---|---|---|
| `ep9_hints` has 0 normal and build dependencies | dependency guard and the xtask check "`ep9_hints` zero-deps" (08 §7) | A1h |
| No engine crate depends on tsrs | `cargo tree -i tsrs_core` is empty for the root workspace (`docs/adr/H26-toolchain-and-unicode.md:40`) | M0 (holds) |
| The ep9-typecheck lock equals the tsrs lock, plus only `ep9-typecheck` and `ep9_hints` | lock guard (`docs/adr/H26-toolchain-and-unicode.md:42`; 08 §7) | A1h |
| A header, hash or kind mismatch, or a file with type errors, drops that file's hints and is counted (S3, S5) | `ep9_hints` and `ep9_frontend` tests, one per drop reason | A1h |
| A hint never changes program output | the hint soundness corpus: 24 hole classes [R12] and 13 tsrs holes [R13], output with hints on equals node (10-quality §2) | A1h |
| HINTED facts never become PROVEN, and each one gets a guard or a fuse | H30 Rules; hint chaos fuzzing (10-quality §4) | A4 |
| A failed HINTED guard follows H32; only speculation exits count. JIT code (only if K-A4H keeps HINTED) deopts to T0 and counts the exit; JIT mode has no PIN. The lowering of the code sets which sites have a SLOWPATH variant (`jitless` lowering only); the isolate preset sets the PIN lifetime. A failed guard in AotOpt code deopts to a T0 frame and counts the exit. A site of `jitless`-lowered code with a SLOWPATH variant takes it after 4 deopts (est., the site budget) in both presets that accept `jitless` (jitless and hybrid). When the function's exits reach the function budget (16, est.), the isolate preset applies. In a hybrid isolate (the hybrid part of H32 and H33, gated at K-A4P), for code of each accepted lowering (`hybrid` and `jitless`: the hybrid accept list), the function is PINned (no AotOpt variant is picked: AotBaseline, else Interp) and the instrumented re-tier starts; its jit code is picked over the PIN, and PIN holds until the re-tier policy lifts it. In a jitless isolate the function is PINned to baseline-AOT (else interp), and that PIN is final for the isolate. A site with 4 guard failures stops using its hint in every mode. A hint failure never throws | H32:23, :29-30, :39-46, :58, :90-93, :104; H33:83; 05-aot.md:149, :309-328; 13-execution-modes.md:132, :153, :305, :333-334, :342; `tests::select::t4_pin_with_two_active_aot_opt_variants_picks_neither` and `t4_pin_in_hybrid_holds_until_the_retier_gives_jit_code` | A4 |
| No codegen reads hints before A4 | review of each A1h change (S9; D18 default b) | A1h |

## A1h scope

A1h is "Hint oracle v0" (`docs/plan/09-milestones.md:211`): after M1 exit, due at M4 exit, AOT lane, 1-3 PM (est.). It does not gate M2, M5 or M7. In month 9 it is docs only (this v0; M9-PLAN-D17). It may start because it reads no AOT frame (M9-PLAN-D24, M9-PRE-A2GATE): at the A2 gate reading, 5 of 6 rows have CI evidence, and row 6 (GC-stress clean with AOT frames) is partial for both the linked and the mapped frame, with its proof in route MC-w40-7-ROOT, due A3. From 06486612 (m8-p5, the A2 gate reading) to 5c271a9d, `crates/ep9_vm/src/unit_loader.rs` has no change, and `crates/ep9_unit` changes only `tests/fuzz_seeds.rs` (+401) and the `tests/golden/root-slots.txt` append of slot 327 (`327 root 35 ArrayValues`).

| # | A1h item | Done when | Source | State at 5c271a9d |
|---|---|---|---|---|
| 1 | `tools/ep9-typecheck` builds (unix) | own `[workspace]`, tsrs crates by path, tsrs's lock and toolchain; `scripts/sync-tsrs.sh` builds it when the dir exists | 09 §3, 01 §11, 05 §6 | not started (no dir; the root `Cargo.toml` excludes it) |
| 2 | `ep9_hints` schema v0 | the reader and writer of this ADR, 0 dependencies | 08 §2, 08 §7 | not started |
| 3 | The join in `ep9_frontend`, no codegen use | S1-S4 on the pre-lowering oxc AST; drops counted | 01 §10 | not started |
| 4 | Gate: join ≥ 99.5% of core node kinds (est.) | measured on HINT-SET | `docs/plan/09-milestones.md:136`, :211 | the prototype joined 99.938% of 408,446 core nodes of TS src/compiler [R13] |
| 5 | SP-HintPrecision report on HINT-SET | any, class-receiver, slot-found and call-target share per group; the D18 off-ramp rule | `docs/plan/09-milestones.md:255`; 05 §5 | not started |
| 6 | SP-RegionCoverage report | applies K-A5v-cov | `docs/plan/09-milestones.md:257`; 10-quality §3 | not started |
| 7 | SP-tsrs-win again | the same check covers ep9-typecheck; on fail, `ep9c build --hints` prints "not supported" on Windows (D2) | `docs/plan/09-milestones.md:253` | not started |
| 8 | The hint soundness corpus | runs on every PR that touches hints | 10-quality §2 | not started |

Not in A1h: the HINTED consumers in `ep9_opt` (A4), K-A4H (A4), the `ep9c build --hints` facts-section write (H37-Q5).

## Consequences

- **Gains.** tsrs changes, its decoder and its Unicode pin stop at the `.ep9h` boundary. The binary form is about 20 B per node; the JSON prototype was about 195 B per node, and type strings would cost about 620 MiB at export [R13].
- **Costs.** A second toolchain world (H26). A file that tsrs and oxc parse differently loses its hints (S3): for example an identifier with a character that Unicode 15.1 (tsrs) does not have and Unicode 18.0 (oxc, M9-PRE-UNICODE) has.
- **Follow-ups.** A1h builds items 1-8. A4 adds the consumers and the K-A4H measurement. The facts-section write follows H35 and doc 13 F-R1.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R13] | The prototype oracle: 99.94% join of 556,724 oxc nodes; the BOM and UTF-16 offset hazard; 6 compile errors against a 15-day-old tsrs; 20 B versus 195 B per node | [../research/13-tsrs-type-oracle.md](../research/13-tsrs-type-oracle.md) |
| [R12] | 24 crafted hole classes pass strict checks and lie at run time | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R17] | oxc_transformer gives lowered TS forms dummy spans, so the join is on the pre-lowering AST | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) |
| tsrs pin | Re-check of H26 Context (`docs/adr/H26-toolchain-and-unicode.md:17`) on 2026-11-18, by git only at 745eb10a (`third_party/tsrs.commit`; no checkout, no build): the tsrs workspace still pins `unicode-id-start = "=1.1.2"` [TSRS Cargo.toml:41], with the comment "1.1.2 uses Unicode 15.1.0" [TSRS Cargo.toml:39]; `tsrs_core` takes it [TSRS crates/tsrs_core/Cargo.toml:26]; the lock has one entry, 1.1.2 [TSRS Cargo.lock:1580-1582]; the toolchain is 1.99.1 [TSRS rust-toolchain.toml:4]. So H26:17 still holds. The root lock has 1.5.0 and no `tsrs_core` | this ADR, Validation |

## Validation

- A1h: the gate and reports of the A1h scope (items 4-7), the guards of the Rules table, `ep9_hints` round-trip tests for each header field and each drop reason, and the `.ep9h` parser fuzz target (10-quality §4: no panics; bad input drops the hints).
- A4: hint chaos fuzzing (its join part from A1h; 10-quality §4) and K-A4H.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H37-Q1 | The byte layout of a record (about 20 B): field widths, the codes of repr category and confidence, the null value of each field | A1h | Fixed-size little-endian records; each code list append-only. The confidence byte matches the per-record confidence of H30-Q1 |
| H37-Q2 | The node-kind code of the key: whose list (oxc AST kinds or tsrs SyntaxKind)? | A1h | An ep9-owned code list in `ep9_hints`; `ep9_frontend` maps oxc kinds to it and `ep9-typecheck` maps tsrs kinds to it |
| H37-Q3 | The decoded-text hash: which function and width? | A1h | The H2 source hash: SHA-256 of the decoded UTF-8 text (H02 F12), so a unit and its `.ep9h` name one source the same way |
| H37-Q4 | The offset unit of (start, end) | A1h | UTF-8 byte offsets of the decoded text after the BOM strip, as oxc spans and the R13 prototype use |
| H37-Q5 | When does `ep9c build --hints` write the HINTED FactSnapshot into the unit `facts` section: at A1h, or with the first consumer? | A1h | A1h writes only the join counts and the SP-HintPrecision report; the facts-section write comes with the first consumer (A4) |
| H37-Q6 | How are the function-entry records of 05-aot.md:229 (parameter and return repr) keyed? | A1h | As L2 function records by function id (doc 13:413), not as S7 (function id, feedback slot) entries |

## Reopens if

- The A1h join gate fails (below 99.5% of core node kinds), or the hint soundness corpus finds a hint that changes output.
- D18 becomes (c) by the SP-HintPrecision off-ramp, or K-A4H trips: HINTED then has no consumer, and only `ep9 check` stays.
- tsrs changes its decoder or its span form so that S1 or S2 no longer holds.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-11-18 | draft | First draft, schema v0 (plan v14 §7.2; M9-PLAN-D25; route A1H-DOCS): the plan row and the 05 §7 layout, S1-S9 (S7 and S8 follow doc 13 F-R1: the HINTED FactSnapshot goes to the unit `facts` section, not to the CompiledUnit hint section), the Freeze table (one part, Schema v0, at A4 start), the A1h scope, Q1-Q5, and the tsrs pin re-check (still `=1.1.2`). README §5 moves H37 from Not started to Draft (the M4-P2 ADR-A0 form) | M9-P4 A1H-TEXT (AOT lane) |
| 2026-11-18 | draft | Route MC-w46-1 (MODES-CHECK-w46; M9-P5 integration fixer r1, DL M9-P5-WPRE-4): Depends on adds H32 (the guard exit of a failed HINTED guard); a Rules row on a failed HINTED guard, per mode as H32 gives it (W-pre check F7), with the hybrid PIN of Codex M9-P5 r1 (fixer r2 7e293497); Open question H37-Q6 (the key of the function-entry records of 05-aot.md:229), and the Freeze cell says Q1-Q6; the A1h scope paragraph adds the 06486612 to 5c271a9d diff of `crates/ep9_unit` and `unit_loader.rs` | M9-P5 integration fixer r1 |
| 2026-11-19 | draft | Route MC-w47-5 (MODES-CHECK-w47, w47-verify part 3): :65 adds the K-A4P gate words of the hybrid part; :70 drops the c583b842..5c271a9d diff sentence; :127 names the hybrid PIN edit 7e293497, which had no Change-log row; Codex M10-P0 r2 (M6-ORCH-1050): :65 says that the lowering sets SLOWPATH and the isolate preset sets the PIN lifetime (a hybrid isolate also takes `jitless` code), and :70 gives the A2 gate reading as M9-PRE-A2GATE does (row 6 partial; A1h reads no AOT frame); Codex M10-P0 r3 (M6-ORCH-1060): :65 says that a `jitless`-lowered site with a SLOWPATH variant takes it after the site budget in both presets that accept `jitless` (doc 13 :132, :153), and the isolate PIN and re-tier policy applies at the function budget | M10-P0 MODES-TEXT-M10 (AOT lane) |
