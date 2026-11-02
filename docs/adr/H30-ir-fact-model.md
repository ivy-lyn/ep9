# H30: IR fact model with provenance

| Field | Value |
|---|---|
| Status | Draft (2026-10-22): the plan row and the merge rule. FactSnapshot v0 already names the three sources (H16); the IR and the merge are not built. SP-Facts (M1) validates the lattice and the ref nodes |
| Fix by | Draft M0 month 3; frozen M1 exit |
| Freeze point | Parts (the Freeze table, README §3.4; H30-Q7): the model at M1 exit; the fused HINTED kinds after the M1 list and the ref-node kinds as append-only lists; the VERIFIED region table at the end of A5, only if it is built |
| Origin | R16-D3; plan status N |
| Crates | `ep9_facts` (`FactSource`, FactSnapshot), later `ep9_opt` (IR facts, ref nodes, graph builder), `ep9_aot` (the AOT merge), `ep9_frontend` (HINTED join, H37) |
| Owner (plan role) | AOT senior (SP-Facts, A0 part 2); CORE T2 owner (the optimizer) ([09 §4](../plan/09-milestones.md#4-spikes)) |
| Validated by | SP-Facts inside Spike C (M1); later hint chaos (A4), mode x tier differential (A3) |
| Plan sources | [02 §6 H30](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [02 §3](../plan/02-architecture.md#3-fact-sources), [04 §6](../plan/04-execution.md#6-shared-optimizer-the-ir-fact-model-h30), [05 §4](../plan/05-aot.md#4-type-trust-levels), [05 §11](../plan/05-aot.md#11-hybrid-mode-feedback-rule-h33), [H16](H16-ics-and-feedback.md) |

## Context

- One optimizer must serve the JIT (T2) and optimized-AOT (A4). Every dual-mode VM studied runs one optimizer fed by different fact sources [R11][R14].
- TS types are unsound by design: 24 crafted hole classes pass strict checks and lie at run time [R12]. So a hint can never become a proven fact (N11).
- AOT code cannot hold a run-local heap address or ShapeId (H8, H12, H35).
- Depends on: H8 (ShapeId rule), H12 (no address identity), H13 (stable indices for RootRef and IntrinsicRef), H16 (FactSnapshot = feedback schema), H24 (debugger rules keep PROVEN facts sound), H31 (fuses), H37 (HINTED entries).
- Depended on by: H32 (guard policy per fact), H33 (RUNTIME overrides), H34 (ref nodes lower to relocation kinds), H35 (unit hint section, link table).

## Decision

Copied from the plan row (02 §6): proven ⊑ predicted{source ∈ RUNTIME, PROFILED, HINTED; confidence}. Precedence RUNTIME > PROFILED > HINTED per fact kind, on a conflict. Merge rule: fused HINTED facts with no runtime counterpart (CHA, enum constant, non-null, slot pre-size) stay. Guard policy per mode in the IR. No raw heap constants: RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst. One graph builder reads bytecode + FactSnapshot. The provenance set is open; `VERIFIED{region}` is reserved for D17 (d) and built only if the D17 rule picks it [R18].

```text
  unknown                         generic code path, no guard
     |
  predicted{source, confidence}   source: RUNTIME | PROFILED | HINTED; needs a Guard (H32)
     |
  proven                          no guard          (proven ⊑ predicted ⊑ unknown; 04 §6)
```

| Source (02 §3) | Producer | Trust | Precedence |
|---|---|---|---|
| PROVEN | ep9's own analysis: never-reassigned module bindings, `const`, scopes without direct eval or `with`, `&`, `|`, `^`, `~`, `<<` and `>>` → int32, `>>>` → uint32 (a double when ≥ 2^31), range analysis, spec invariants | proven | always holds |
| RUNTIME | FeedbackVector and IC data of T0, T1 and baseline-AOT ICs (H16, H33) | predicted | 1 (highest) |
| PROFILED | `.ep9prof` from a training run | predicted | 2 |
| HINTED | `.ep9h` from ep9-typecheck, joined in `ep9_frontend` | predicted, HINT trust | 3 (lowest) |

| IR node (04 §6) | Meaning | Lowers to |
|---|---|---|
| RootRef(n) | Isolate root by stable index (H13) | RootIndex(n) |
| IntrinsicRef(j) | Realm intrinsic by stable index | Load from the current realm's intrinsics table (H13, H22) |
| ShapeRef(s) | A shape used in a guard | ShapeId immediate only for snapshot root shapes; else a CellRef (H8) |
| CellRef(i) | RW IC or link cell | IcCell(i) or LinkSlot(i) |
| UnitConst(i) | Constant from the unit's constant pool | pc-relative rodata load |
| Guard(kind, fact) | Tag, shape, elements kind, call target or non-null check | Policy from H32 per mode |
| Assume(fuse) | Global invariant | GuardFuse check or InvalidatingFuse dependency (H31) |

**Merge rule (04 §6, 05 §11).** Precedence applies per fact kind. A slot with RUNTIME facts ignores PROFILED and HINTED facts of the same kind (shape, call target, representation). HINTED facts that feedback cannot give stay beside RUNTIME facts: CHA and enum constants under a fuse (H31), non-null, slot pre-size.

**As built (e719ffa).** `ep9_facts::FactSource` has the three predicted sources as bytes `Runtime = 0`, `Profiled = 1`, `Hinted = 2` (`crates/ep9_facts/src/bits.rs:122-131`). The source is one header byte per snapshot (`crates/ep9_facts/src/snapshot.rs:60`; header byte 48, `crates/ep9_facts/src/codec.rs:6`), and the decoder refuses any other value (`crates/ep9_facts/src/codec.rs:287`). There is no confidence field and no IR yet.

## Rules

| Rule | Check | From |
|---|---|---|
| A FactSnapshot names exactly one source of RUNTIME, PROFILED or HINTED; any other byte refuses the snapshot | `ep9_facts` test `header_errors`: source byte 3 gives `BadValue("fact source")` (`crates/ep9_facts/tests/encoding.rs:184-185`); `tests/schema.rs` (schema fingerprint, H16) | M0 wk4 |
| TS types never become PROVEN; HINTED facts always get a Guard or a fuse | M1: Spike C / SP-Facts (lattice); hint soundness corpus (10-quality §2) and hint chaos fuzzing (10-quality §4) | M1; A1h; A4 |
| The IR holds no raw heap constant; only ref nodes (the five of the Decision today; an append-only list, Freeze table) | M1: Spike C / SP-Facts (ref nodes); the unit writer rejects HeapConst (H34, 04 §5) | M1; A2 |
| ShapeRef lowers to an immediate only for a snapshot root shape | unit golden tests (H8 rule row `docs/adr/H08-object-header-and-shapeid.md:89`) | A2 |
| RUNTIME > PROFILED > HINTED per fact kind; fused HINTED facts with no runtime counterpart stay | M1: Spike C / SP-Facts (one kernel from a PROFILED FactSnapshot file gives identical output); profile staleness suite (10-quality §2) | M1; A3 |
| One graph builder reads bytecode + FactSnapshot for JIT and AOT | M1: Spike C / SP-Facts; mode x tier differential (10-quality §4) | M1; A3 |
| The guard policy is chosen per mode in the IR, not in the backend | M1: Spike C / SP-Facts; H32 rules | M1 |

## Consequences

- **Gains.** One optimizer for T2 and optimized-AOT; runtime feedback, profiles and hints are interchangeable inputs because the FactSnapshot schema equals the feedback schema (H16). Unsound hints cannot cause a type confusion without a failed guard.
- **Costs.** Every predicted fact costs a guard. Ref nodes add one load for a non-root shape or a link cell in AOT code.
- **Follow-ups.** SP-Facts (M1). A speed-only miss of SP-Facts after its time box is recorded here and does not block M1 exit (README §2, 09 §4). D17 at M7 exit decides if VERIFIED is built [R18].

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R14] | JSC DFG keeps proven AbstractValue apart from per-node predictions; Warp builds from a feedback snapshot | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R11] | Dart, Graal, .NET, ART: one optimizer per VM; modes differ in fact sources and in deopt | [../research/11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R16] | D3: proven or predicted{source, confidence}; policy in the IR; no raw heap constants | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R12] | 24 crafted TS hole classes pass strict checks and lie at run time | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R18] | VERIFIED{region} facts, keyed by region version, rank above RUNTIME inside a region while its fuses hold | [../research/18-sound-regions.md](../research/18-sound-regions.md) |

## Validation

- M1: SP-Facts inside Spike C: the Spike C IR uses the lattice and the ref nodes; one kernel compiled from a PROFILED FactSnapshot file gives identical output at ≥ 90% of live-feedback speed (est.) (09 §2 M1 exit item 10). A correctness or structure failure reopens H30-H34.
- A3/A4: profile staleness, mode x tier differential, hint chaos.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H30-Q1 | Where does `confidence` live? FactSnapshot v0 has a source per snapshot and no confidence field | M1 (H16 freeze) | A per-record confidence byte for PROFILED and HINTED only; adding it bumps `FACT_SCHEMA_VERSION`. **Proposal 2026-10-27 (M4-P2 ADR-A0; C-05, C7):** as proposed: 1 byte per record of a PROFILED or HINTED snapshot, in the one month-6 `FACT_SCHEMA_VERSION` bump (V9-D10). H32 "HINTED: at lower confidence" reads this byte (H32-Q1). **As built 2026-11-02 (M6-P0 SCHEMA6, FACT_SCHEMA 2):** one confidence byte per record of a PROFILED or HINTED snapshot, after the record's kind and flags bytes; a RUNTIME record has none, so whether the byte is there follows the snapshot's source byte (`ep9_facts::RecordMeta`, `FactSnapshot::insert_with`; the "Record" line of the schema text). 0 = no confidence given, 1 lowest to 255 highest. Nothing reads it yet (the IR is not built), and the VM exporter writes RUNTIME snapshots only, so nothing writes it either |
| H30-Q2 | "The provenance set is open": a new source byte (VERIFIED = 3) is refused by a v1 decoder. Reserve it now or bump the schema when D17 (d) is picked? | M1 exit (freeze) | Reserve the value in the schema text at the M1 freeze; the decoder keeps refusing it until built. **Filled 2026-10-27 (M4-P2 ADR-A0; plan v9 §4.1 task 3, C-05): VERIFIED = 3 is reserved as a name of the provenance set, and it is never a FactSnapshot source.** A FactSnapshot keys facts by (function, slot) (H16), and VERIFIED facts are keyed by region version (H30-Q5), so they get their own region table, never FactSnapshot records. The decoder keeps refusing source byte 3 (`header_errors`, `crates/ep9_facts/tests/encoding.rs:184-185`), and no schema bump adds a VERIFIED field |
| H30-Q3 | The per-fact-kind precedence table: which fact kinds exist, and which HINTED kinds count as "fused, no runtime counterpart" beyond CHA, enum constant, non-null, slot pre-size | M1 (SP-Facts) | Fact kinds = H16 slot payloads + the 4 fused kinds named by the plan |
| H30-Q4 | Merging two snapshots of one function (RUNTIME + HINTED) in the JIT: snapshot time and thread | M1 (SP-Facts) | Main-thread snapshot as Warp (04 §6); merge off-thread. **Proposal 2026-10-27 (M4-P2 ADR-A0; C-05):** one `ep9_facts::merge(sources) -> FactSet` serves the JIT and `ep9_aot` (`ep9_facts` depends on `ep9_bytecode` only, so both sides can call it). The JIT takes the main-thread snapshot and merges off-thread; compile threads never read live heap objects (H21 P7) |
| H30-Q5 | How a VERIFIED fact is keyed (by region version, not by bytecode site) and how it ranks against RUNTIME [R18] | M7 exit (D17 rule) | Not built unless D17 picks (d) |
| H30-Q6 | Which spec invariants and range facts count as PROVEN, and the list of Math range facts (H36 CR results) | M7 (T2) | A closed list in `ep9_opt`, each with a differential test |
| H30-Q7 | README §3.4 asks a Freeze table for a row built after M1 exit | M1 exit | The whole row freezes at M1 exit; the SP-Facts speed number may be recorded later (README §2). **Filled 2026-10-27 (M4-P2 ADR-A0; MODES-D1): the Freeze table below** |

## Freeze table

README §3.4 form (H30-Q7; plan v9 §4.1 task 9; the design is [13](../plan/13-execution-modes.md) §4 and §5.4, MODES-D1). A gated part freezes at the gate that ends its build.

| Part | What it holds | Freeze point | After the freeze point | Check |
|---|---|---|---|---|
| Model | Decision and Rules: the lattice, the precedence per fact kind, the merge rule, the ref-node rule (no raw heap constant) and the meaning of each ref node, one graph builder, the guard policy in the IR; the fills of Q1 and Q4, and the M1 list of the Q3 fill (the fact kinds and fused HINTED kinds it names by M1 exit); the provenance set: the FactSnapshot sources 0 RUNTIME, 1 PROFILED, 2 HINTED, and VERIFIED = 3 as a reserved name only (Q2; a new FactSnapshot source would be a schema change, H16) | M1 exit | Frozen-part rules (README §3.2) | SP-Facts (M1 exit item 10). A speed-only miss after the time box is recorded, not blocking (README §2). SP-Facts measures PROFILED facts under JIT lowering only, so it is not a jitless-lowering number; the A4 gate names that measure (MC-w13-23) |
| Fused HINTED kinds and ref-node kinds (append-only) | The fused HINTED kinds ("no runtime counterpart") after the M1 list of the Q3 fill: no codegen reads hints before A4 (`docs/plan/01-front-end.md:238`; hint consumers come at A4, `docs/plan/02-architecture.md:230`), so kinds found in A4 come by append. The ref-node kinds: RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst, in this order; a new kind names its lowering (H34) in its fill row (MC-w14-4) | None | Append only; a kind's meaning never changes | when built: the unit writer refuses an unknown ref node (A2); the merge drops an unknown fused kind (A4) |
| VERIFIED region table (gated) | The region table format and its ranking against RUNTIME (Q5) | The end of A5 (the K-A5v decision), only if the D17 rule picks (d) | Frozen-part rules | A5 gates |

## Reopens if

- SP-Facts fails a correctness or structure gate: lattice, ref nodes or identical output (09 §4).
- K-A4H trips (hints dropped) or D18 becomes (c): the HINTED source then has no consumer, but the lattice stays.
- The D17 rule picks (d) at M7 exit: VERIFIED is built (a fill of H30-Q2 and H30-Q5).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-22 | draft | First draft from 02 §6 row H30, 02 §3, 04 §6 and 05 §11, with the FactSource bytes of FactSnapshot v0 as built; confidence, VERIFIED and the fact-kind table are Open questions | P3-ADR-JIT agent (M2-P3) |
| 2026-10-22 | rewrite | PROVEN row: "bitwise ops → int32" was wrong for `>>>`, which gives uint32 (ToUint32, ECMAScript §13.9; the interpreter boxes it as a double: `crates/ep9_vm/src/interp/ops.rs:142`, `crates/ep9_vm/src/conv.rs:397`; node: `-1>>>0` is 4294967295). The row now lists the int32 operators and gives `>>>` as uint32. The same fix is in plan 02 §3 and 04 §6, which this row copies. Codex review of the P3 merge (M2-P3-RF1-H30) | P3 review fix (orchestrator) |
| 2026-10-27 | fill (H30 is Draft) | Freeze table and Open-question text (plan v9 §4.1 tasks 2, 3 and 9; MODES-D1, MODES-D3, C-05): the Freeze point field names the parts; a Freeze table (the model and the provenance set at M1 exit, with the SP-Facts note of MC-w13-23; the VERIFIED region table at the end of A5); Q2 filled (VERIFIED = 3 reserved as a name, its own region table, never FactSnapshot records); Q1 and Q4 proposals; Q7 filled. Every check keeps its result | M4-P2 ADR-A0 (VM lead role) |
| 2026-10-27 | clarify (H30 is Draft) | MODES-CHECK-w14 finding MC-w14-4 (U3; 13 §5.4; MODES-D1: a part freezes at the gate that ends its build): the Q3 fill is split, so the Model part (M1 exit) holds only the M1 list of fact kinds and fused HINTED kinds, and the fused kinds after it form an append-only list (no codegen reads hints before A4, so kinds found in A4 would otherwise need a rewrite); the ref-node kinds become an append-only list too (the Rules row and the Freeze point field follow). No check, value or ref node changes | M4-P3 integration fixer (docs part A) |
| 2026-11-02 | fill (as built; H30 is Draft) | H30-Q1 as built (M6-P0 SCHEMA6; plan v11 §2.3 "FACT_SCHEMA 2 contract", §2.5; V10-D58; the H16 2026-11-02 rewrite (version) row): an "As built" note in the Q1 row: the per-record confidence byte of a PROFILED or HINTED snapshot (none in RUNTIME), 0 = none given, else 1-255; no reader and no writer yet. Review record (README §3.1; plan v11 rule 21): rows 1-4 come from the M6-P0 reviewer and fixer | M6-P0 SCHEMA6 agent |

## Review record

Records of the Change-log rows since m0-exit (README §3.1; plan v11 rule 21 (a): from M6-P0 every Change-log row of H30 gets rows 1-4 in its own phase). [2026-11-02, M6-P0 RECORDS-M6] The 2 Change-log rows dated 2026-10-26 to 2026-10-28 that had no complete record (the freeze dry-run list) get their records now (plan v11 §2.10, rule 21 (a)): one set per row; rows 1 and 4 by M6-P0 RECORDS-M6 are the rows of the table below, each set in Change-log order; rows 2 and 3 come from the M6-P0 integration fixer (from the orchestrator's Codex docs slice M6ROWS) and the Approval from the orchestrator.

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer (M4-P2 amendment: the 2026-10-27 Freeze table and Open-question row, ADR-A0) | M6-P0 RECORDS-M6 (Claude, the records role of the GC chain; plan v11 §2.10, rule 21 (a) (b) (e); wrote none of the change: no H30 text, no Change-log row, no month-4 code, test or golden, no row 2 or 3 and no Codex text) | Checked at a47c4e6e against 77ae1f37 (ADR-A0; docs only). (1) The H30 diff is as the row says: the Freeze point field names the parts; a Freeze table (Model at M1 exit, with the SP-Facts note of MC-w13-23; the VERIFIED region table at the end of A5, only if the D17 rule picks (d)); Q2 filled (VERIFIED = 3 is a reserved name, with its own region table, never FactSnapshot records); Q1 a proposal (one confidence byte per record of a PROFILED or HINTED snapshot, in the one month-6 schema bump) and Q4 a proposal (one `ep9_facts::merge` for the JIT and `ep9_aot`); Q7 filled. The Q2 fill's claim held at 77ae1f37: `crates/ep9_facts/tests/encoding.rs` `header_errors` asserts that source byte 3 decodes to `BadValue("fact source")`. The Decision and the Rules rows are not in the diff. At a47c4e6e the M6-P0 SCHEMA6 role builds the Q1 byte (plan v11 §2.5). (2) Kind test (README §3.3, quoted): clarify "Text only. No check, golden file, test or value changes result. The PR changes no code"; fill "Adds a rule or value where the ADR had an Open question or a part the plan defers. Every existing Rules check, test and golden file keeps its result"; rewrite "Anything else", in particular "a Decision statement or Rules row changes or goes away" and "a chosen option is replaced"; "If they disagree, the kind is rewrite". Applied: Q2 and Q7 are Open questions answered, the table is the part README §3.4 asks for, Q1 and Q4 are proposals; every Rules check keeps its result -> fill. Author: fill. **Confirmed** | read at a47c4e6e (branch m1m6/p0-records, mac a64): `git show` of the named commits (77ae1f37; `crates/ep9_facts/tests/encoding.rs` at 77ae1f37) |
| 4 Data (M4-P2 amendment: the 2026-10-27 Freeze table and Open-question row, ADR-A0) | M4-P2 ADR-A0 (VM lead role; runs only; it wrote the change); copied by M6-P0 RECORDS-M6 (plan v11 §2.10) | Docs only (77ae1f37). No crate was built and no test262 run (none in the ADR-A0 acceptance). ADR-A0 at its role head 778c12d: ADRLINE exit 0, checked=536 ok=536 fixed=0 unfixable=0 (515 at base); `cargo test --locked -p xtask` 69 + 2 (7 ignored) + 18 pass, 0 fail; `adr-lint --trailers-only` PASS from 12cfc61 (5 commits) and from m0-exit (156 commits); README Counts (10 Draft, 1 Not started, 25 Accepted, 1 Frozen) equal the Status lines; path, email and token scan of the diff clean; every new Change-log row has a README §3.3 kind word or `draft` | ADR-A0 role report scratchpad m1m4/p2/merge/role-reports.md (first section); decisions log §14.4 M4-P2-ADRA0 |
| 1 Reviewer (M4-P3 amendment: the 2026-10-27 MODES-CHECK-w14 MC-w14-4 row, M4-P3 integration fixer docs part A) | M6-P0 RECORDS-M6 (Claude, the records role of the GC chain; plan v11 §2.10, rule 21 (a) (b) (e); wrote none of the change: no H30 text, no Change-log row, no month-4 code, test or golden, no row 2 or 3 and no Codex text) | Checked at a47c4e6e against e902a537 (M4-P3 integration fixer, docs part A; docs only). (1) The H30 diff: the Model part now holds "the ref-node rule (no raw heap constant) and the meaning of each ref node" and "the M1 list of the Q3 fill", where it held "the five ref nodes" and "the fills of Q1, Q3 and Q4"; a new part "Fused HINTED kinds and ref-node kinds (append-only)" (RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst, in this order; later fused HINTED kinds by append); the Rules row "The IR holds no raw heap constant; only the five ref nodes" now reads "only ref nodes (the five of the Decision today; an append-only list, Freeze table)"; the Freeze point field follows. The plan cites hold at e902a537: `docs/plan/01-front-end.md` line 238 says "No codegen reads hints before A4 (D18 default b)", and `docs/plan/02-architecture.md` line 230 puts hint consumers at A4. The reason is sound (MODES-D1). (2) Kind test (README §3.3, quoted): clarify "Text only. No check, golden file, test or value changes result. The PR changes no code"; fill "Adds a rule or value where the ADR had an Open question or a part the plan defers. Every existing Rules check, test and golden file keeps its result"; rewrite "Anything else", in particular "a Decision statement or Rules row changes or goes away" and "a chosen option is replaced"; "If they disagree, the kind is rewrite". Applied: a Rules row changes (the closed set "only the five ref nodes" becomes an open list), and the Model part, frozen at M1 exit, no longer holds the later fused kinds: a decided choice of what freezes at M1 exit is replaced (the M4-P4 precedent: Codex D2-2 and the M4-P4 integration fixer C re-kinded the H34 MC-w15-3 row rewrite because it moved the freeze point of a part and its removal condition (H34 Change log, the 2026-10-27 D2-2 row)). Author: clarify ("No check, value or ref node changes"; the commit message says "Each ADR is Draft, so each row is a clarify row"), but Draft status does not set the kind: README §3.3 applies to Draft rows too, as the M4-P4 re-kinds of H21 and H34 show. **Not confirmed: rewrite** (the reviewer disagrees, so README §3.3 makes it rewrite; pre-freeze churn, H30 is Draft, G8 does not count it). The kind cell is outside this role's grant: a fixer need with the exact cell text (M6-P0 RECORDS-M6 report) | read at a47c4e6e (branch m1m6/p0-records, mac a64): `git show` of the named commits (e902a537); docs/plan/01-front-end.md and docs/plan/02-architecture.md at e902a537 |
| 4 Data (M4-P3 amendment: the 2026-10-27 MODES-CHECK-w14 MC-w14-4 row, M4-P3 integration fixer docs part A) | M4-P3 integration fixer, docs part A (runs only; it wrote the change); copied by M6-P0 RECORDS-M6 (plan v11 §2.10) | Text only (e902a537: H30-H34, +24 -8, `docs/adr/` only; ADR trailer H30-H34). The docs fixer's 4 commits on `m1m4/p3-fixdocs` merged as 35c9d49 (decisions log M4-P3-FIX-2); I found no acceptance log of the docs part in the scratch records. Later heads that hold the commit: the part B2 ADRLINE 5eaf080a (after: checked 657, ok 657, fixed 0, unfixable 0) and the part B2 T262 at 517a13d (exec and eager 37,829 / 53,616, 0 new) (decisions log M4-P3-FIX-5) | decisions log §14.5 M4-P3-FIX-2 and M4-P3-FIX-5; `git merge-base --is-ancestor e902a537 5eaf080a` |
