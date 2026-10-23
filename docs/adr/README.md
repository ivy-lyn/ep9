# ep9 architecture decision records (ADRs)

- Each hard decision H1-H37 in [02-architecture §6](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37) gets one ADR in this directory.
- An ADR copies the plan's choice, adds the testable rules, names the evidence and the spike that validates it, and states when it reopens.
- Status runs Draft -> Accepted -> Frozen. After M1 exit, no H row changes without an approved ADR amendment (M1 exit item 6 in [09-milestones §2.2](../plan/09-milestones.md#22-scope-and-exit-criteria)).
- G8 measures the process: the ADR log shows 0 rewrites of H rows after M1 exit.
- Start a new ADR from [TEMPLATE.md](TEMPLATE.md). Decisions D1-D21 (decided 2026-10-20 under the user's delegation) are logged in [../decisions-log.md](../decisions-log.md), not here.

## 1. Files

| Item | Rule |
|---|---|
| One file per H row | `Hnn-<slug>.md`, two-digit number, slug in kebab case. The planned names are in the index (§5) |
| Source of truth | Until an ADR is Accepted: the plan row in 02-architecture §6 and the plan section that owns the detail. A Draft copies them; if a Draft and the plan disagree, fix the Draft or file a plan fix. From Accepted on, the ADR is the record, and the plan row changes with it |
| New design | Only where the plan leaves a gap. Each gap is listed under "Open questions" and is not part of the decision until it is resolved |
| Evidence | Tags [R01]-[R18] link to `../research/`. [P1] is a plan v1 probe that a named M0 spike checks again |
| Crates | Each ADR names the crates it governs. The ADR lint (below) uses this line |

## 2. Status and life cycle

```
 Not started --write--> Draft --review, merge by "Fix by"--> Accepted --freeze point--> Frozen
                          ^                                     |                         |
                          +-- a spike gate fails (reopen) ------+                         |
                          +-- an approved amendment (benchmark) <-------------------------+
```

| Status | Meaning | Who moves it (P-1, §3.2) | Change rule |
|---|---|---|---|
| Not started | No file yet | Owner of the row (plan role, §4) | n/a |
| Draft | Written, under review. Content may change | Author | Free edits in normal PRs |
| Accepted | Merged as an ADR. The "Fix by" point is met (M0 exit item 9: every "Fix by M0" row is merged) | The orchestrator (lead role) merges when the review record (§3.1) is complete | A PR that names the ADR, adds a Change-log line with its kind (§3.3) and has the review record |
| Frozen | Past the part's freeze point (§3.4), or past M1 exit | The orchestrator at the milestone review. The milestone report tells the user | Only an approved amendment backed by a benchmark or gate output ([02 §6](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37)), approved as in §3.2 |

- **Reopen.** A failed spike gate reopens the named ADRs before the milestone continues ([09 §4](../plan/09-milestones.md#4-spikes)). The ADR goes back to Draft. Its Change log names the gate and the result.
- **Speed-only miss.** If only the SP-Facts speed number misses after the time box, the H30 ADR records the result and the exit proceeds ([09 §4](../plan/09-milestones.md#4-spikes)).
- **Freeze points from the plan.** H20 at M0 exit. H17-H19 at M1a. H2, H14, H16 and H29-H34 at M1 exit. H35 at the A3 gate. H37 at A4 start. Every other row is frozen at M1 exit.

## 3. Amendments

1. Open a PR that edits the ADR's Decision section and adds a dated Change-log line.
2. Attach the data: a benchmark for a Frozen row (02 §6), or the failed gate output for a reopen.
3. Name the change kind: **clarify** (no behavior change), **fill** (resolves an Open question or a part the plan defers), or **rewrite** (changes a decided choice).
4. Update the plan row in the same PR, or file a plan fix with the plan owner.
5. Update the status column of the index (§5).

Process questions P-1 to P-4 were decided on 2026-10-20 under the user's delegation ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction); [12-governance](../plan/12-governance.md)). §3.1-§3.4 and §4 hold the result.

### 3.1 Review record (P-1)

Accept, and every amendment of an Accepted or Frozen ADR, need all of these. The PR body records them.
1. A reviewer agent that did not write the change. It checks the Decision and Rules against the plan row and the evidence.
2. A Codex adversarial review (`/codex:adversarial-review --model gpt-6.1-sol`, run in the background) of the PR.
3. An agent that did not write the change verifies each Codex finding through the full control flow and sorts it: (a) serves the ADR's goal, (b) a defect the PR introduced, (c) neither. Only (a) and (b) change the PR. A (c) finding goes to Open questions or to an issue.
4. The data the change needs: the spike gate output for Accept and reopen, a benchmark or gate output for a Frozen part.

Phase-merged work with no per-ADR PR keeps the record in the ADR's `## Review record` section (M2-ACCEPT-RECORD).

### 3.2 Who approves (P-1)

| Change kind | Draft | Accepted | Frozen part |
|---|---|---|---|
| clarify | Author | Orchestrator + review record | Orchestrator + review record |
| fill | Author | Orchestrator + review record | Orchestrator + review record + gate output |
| rewrite | Author | Orchestrator + review record + data | Orchestrator + review record + benchmark + a second reviewer agent from another plan role. A dated line in the decisions log. G8 counts it |

- **D items and goals.** The agent team decided D1-D21 under the user's delegation of 2026-10-20 ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)). An ADR change that would alter a D item happens only through that item's written re-check rule ([11-decisions §2](../plan/11-decisions.md#2-how-the-decisions-depend-on-each-other)) or an owner override, and the decisions log gets a dated entry. A change that would alter a goal G1-G15 goes into the next report to the user as a statement; the row stays as it is meanwhile, and no work waits.
- The orchestrator never approves its own change. If it wrote the change, a second reviewer agent from another plan role approves.

### 3.3 Change kinds and G8 (P-2)

| Kind | Test (all must hold) |
|---|---|
| clarify | Text only. No check, golden file, test or value changes result. The PR changes no code |
| fill | Adds a rule or value where the ADR had an Open question or a part the plan defers. Every existing Rules check, test and golden file keeps its result |
| rewrite | Anything else. In particular: a Decision statement or Rules row changes or goes away; a value that code, data or a file format depends on changes (bit pattern, size, offset, enum value, register, section id, version rule, API signature); a check now accepts what it rejected, or rejects what it accepted; a chosen option is replaced |
| reopen | A failed spike gate before the part's freeze point. After the freeze point a reopen is a rewrite |

- The author proposes the kind in the Change-log line. The reviewer agent confirms it. If they disagree, the kind is **rewrite**.
- **G8 counts** the Change-log lines of kind rewrite on a Frozen part, dated after M1 exit. Rewrites of a part before its freeze point are not counted; the milestone report lists them as "pre-freeze churn".

### 3.4 Parts built after M1 exit (P-3)

An ADR whose row is built after M1 exit lists its parts, each with its own freeze point, in a "Freeze" table that replaces the single "Freeze point" field. Before a part's freeze point, a change to it is a normal Accepted-state change (§3.2 "Accepted" column, kind fill or rewrite) and G8 does not count it. After the freeze point, §3.2 "Frozen part" applies.

| Row | Part frozen at M1 exit (or earlier, §2) | Part built later | That part's freeze point |
|---|---|---|---|
| H3 | Header fields and TS flags | BytecodeGen lowering table | M4 exit |
| H13 | Snapshot rules, stable index lists | Snapshot build (A1) | M4 exit |
| H21 | Design (ADR at M1) | Build | M7 start ("functional before M7") |
| H24 | Rules | AOT parts | A2 gate |
| H35 | (none: the whole row freezes at the A3 gate) | Writer, loader | A3 gate |
| H37 | (none) | Schema v0 (A1h) | A4 start |

## 4. Ownership, CODEOWNERS and the ADR lint

- **Roles.** The plan names roles: lead/front end, GC lead, VM lead, JIT/platform lead, runtime lead, AOT senior ([09 §8](../plan/09-milestones.md#8-m0-first-month-week-by-week)). Under D3 (decided 2026-10-20), AI agents led by an orchestrator agent fill these roles. Each ADR keeps the plan role as its owner.
- **CODEOWNERS (P-4, decided 2026-10-20).** The plan puts CODEOWNERS on the contract crates: `ep9_value`, `ep9_heap`, `ep9_bytecode`, the `ep9_vm` core, `ep9_frontend` lowering, `ep9_masm`, `ep9_baseline`, `ep9_opt`, `ep9_unit`, `ep9_aot`, `ep9_facts` ([10-quality §6](../plan/10-quality.md#6-agent-workflow)).
  - `.github/CODEOWNERS` names the owner account on these crates (whole crates until `ep9_vm` and `ep9_frontend` split), on `ep9_platform`, on `docs/adr/`, on the decision files and on the build and CI files.
  - Branch protection does not require code-owner review. Agents open PRs from the owner account, and GitHub does not let an author approve their own PR ([12-governance §4.3](../plan/12-governance.md#43-why-codeowners-is-not-enforced)).
  - A PR on these paths needs the contract review instead: the §3.1 review record plus a check against the governing ADR's Rules table ([12-governance §4.2](../plan/12-governance.md#42-review-of-a-pr)).
  - Reopens if agents get their own GitHub identity (a GitHub App): then a ruleset can require the owner's review.
- **ADR lint.** Every PR that changes an H-row crate must reference an ADR ([10-quality §5](../plan/10-quality.md#5-other-ci-checks); xtask in [08-repo §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks)). The lint reads the "Crates" line of each ADR.

## 5. Index of H1-H37

"Fix by" is copied from 02-architecture §6. Plan status: U = unchanged from plan v1, E = extended, N = new.

| ID | Decision | Fix by | Plan | ADR status | File |
|---|---|---|---|---|---|
| H1 | Front-end boundary | M0 wk3 | E | Accepted (2026-10-22; review record complete, M2-P4-ACCEPT; was Draft (2026-10-21)) | [H01-front-end-boundary.md](H01-front-end-boundary.md) |
| H2 | Bytecode is a serialized ABI | M0 wk3; determinism CI M0; Spike C checks before M1 exit | E | Draft (2026-10-21; Q5 filled: slot kinds = H16 v0) | [H02-bytecode-abi.md](H02-bytecode-abi.md) |
| H3 | TS runtime semantics in BytecodeGen; TS flags in module identity | Header fields M0 wk3; ADR M0 exit; built M4 | N | Accepted (2026-10-22; review record complete, M2-P3-ACCEPT; was Draft (2026-10-21)) | [H03-ts-runtime-semantics.md](H03-ts-runtime-semantics.md) |
| H4 | Lazy-compile contract | M0 | U | Draft (2026-10-21) | [H04-lazy-compile-contract.md](H04-lazy-compile-contract.md) |
| H5 | Compiler, interpreter and source always present | M0 rule (wk3) | N | Accepted (2026-10-22; review record complete, M2-P4-ACCEPT; was Draft (2026-10-21)) | [H05-always-linked-compiler.md](H05-always-linked-compiler.md) |
| H6 | Value encoding | M0 wk2 | U | Accepted (2026-10-22; review record complete, M2-P4-ACCEPT; was Draft (decided 2026-10-20)) | [H06-value-encoding.md](H06-value-encoding.md) |
| H7 | Address space and code range | M0 wk2 (SP-VA) | E | Draft (rewritten 2026-10-21: one cage per isolate) | [H07-address-space-and-code-range.md](H07-address-space-and-code-range.md) |
| H8 | Object header and ShapeId | M0 wk2 | E | Draft (Q1 and Q3 filled 2026-10-21: shape region, word0 flag bits; Q2 filled 2026-10-22: no shape-guarded load in Spike B) | [H08-object-header-and-shapeid.md](H08-object-header-and-shapeid.md) |
| H9 | Pages and side metadata | M0 wk2 | U | Accepted (2026-10-22; review record complete, M2-P4-ACCEPT; was Draft (Q4 filled 2026-10-21)) | [H09-pages-and-side-metadata.md](H09-pages-and-side-metadata.md) |
| H10 | Rooting | M0 wk2, after SP-GC | E | Accepted (2026-10-22; review record complete, M2-P4-ACCEPT; was Draft (Q2 filled 2026-10-21: gc-lint, Local check)) | [H10-rooting.md](H10-rooting.md) |
| H11 | Barrier and atomics | API M0; slow paths S1/S2; unit field A2 | E | Accepted (2026-10-22; review record complete, M2-P4-ACCEPT; was Draft (decided 2026-10-20; Q3 filled 2026-10-21)) | [H11-barrier-and-atomics.md](H11-barrier-and-atomics.md) |
| H12 | No address identity | M0 | U | Draft (Q1-Q3 filled 2026-10-21; Q1 filled again 2026-10-23, M3-P1 B-COLLECTIONS: the identity hash is built) | [H12-no-address-identity.md](H12-no-address-identity.md) |
| H13 | Snapshot readiness and stable indices | Rules M0; index lists M1; built M4 (A1) | E | Draft (2026-10-21; H13-Q3 filled 2026-10-23, M3-P1 SP-UNIT: the native runtime table is an append-only list with a golden) | [H13-snapshot-stable-indices.md](H13-snapshot-stable-indices.md) |
| H14 | Frames, calls, dispatch slots, pc→code registry | M0 wk3; Spike B (M0 month 2); SP-Unit (M3-P1, moved by M2-D21-FAIL); Spike C (M1) | E | Draft (2026-10-21; feedback vector via fp-4 filled; H14-Q1 filled, FRAME 2; reopened 2026-10-22 by the Spike B gate: H14-Q5 filled, placement (b), JIT frames on the VM register stack; H14-Q2 filled: the caller wraps) | [H14-frames-and-dispatch.md](H14-frames-and-dispatch.md) |
| H15 | Interpreter dispatch | M0 wk2 | E | Draft (reopened twice 2026-10-21; release expansion: sibling calls on every target; reopened 2026-10-22 for x86-64 on Zen 3 (M2-P3-SPD-D12), H15-Q2 due before the M0 exit; Q3 and Q4 filled: cgoto.c by zig cc 0.17.0) | [H15-interpreter-dispatch.md](H15-interpreter-dispatch.md) |
| H16 | ICs and feedback; feedback schema = FactSnapshot schema | Slot kinds M0 wk4; frozen M1 exit | E | Draft (2026-10-21; slot kinds H16 v0, FactSnapshot v0, T0 named/global IC v0 and epoch saturation built) | [H16-ics-and-feedback.md](H16-ics-and-feedback.md) |
| H17 | Object layout | M1 (frozen at M1a) | U | Draft (2026-10-21; object layout v0 on shapes and accessors built) | [H17-object-layout.md](H17-object-layout.md) |
| H18 | Elements kinds | M1a | U | Draft (2026-10-21, v0 notes) | [H18-elements-kinds.md](H18-elements-kinds.md) |
| H19 | Strings | M1a | U | Draft (2026-10-21; strings v0 and symbols v0 built) | [H19-strings.md](H19-strings.md) |
| H20 | Builtin ABI | Frozen M0 exit | U | Draft (2026-10-21; ABI v1 draft, builtin tables and ids, P3 builtins and Cx::initial_name built; Q4 partly settled; M3-P1: R5 and Q2 filled (realms), the clock, collection and realm `Cx` methods, the Date, collections and JSON modules) | [H20-builtin-abi.md](H20-builtin-abi.md) |
| H21 | Shape transitions under concurrent readers | ADR M1; built M5; functional before M7 | U | Not started | `H21-shape-transitions.md` |
| H22 | Isolates and realms | M0 wk3 (D14) | E | Draft (2026-10-21; M3-P1 REALMS: N realms per isolate built, H22-Q3 and H22-Q7 filled) | [H22-isolates-and-realms.md](H22-isolates-and-realms.md) |
| H23 | Limits and failure policy | M0 wk3 | E | Accepted (2026-10-22; review record complete, M2-P5-ACCEPT; was Draft (2026-10-21)) | [H23-limits-and-failure-policy.md](H23-limits-and-failure-policy.md) |
| H24 | Debug and profiling hooks | M0 rules; AOT parts A2 | E | Accepted (2026-10-22; review record complete, M2-P5-ACCEPT; was Draft (2026-10-21)) | [H24-debug-and-profiling-hooks.md](H24-debug-and-profiling-hooks.md) |
| H25 | Sandbox readiness | M0 | U | Accepted (2026-10-22; review record complete, M2-P5-ACCEPT; was Draft (2026-10-21)) | [H25-sandbox-readiness.md](H25-sandbox-readiness.md) |
| H26 | Toolchain and Unicode | M0 wk1 | U | Accepted (2026-10-22; review record complete, M2-P5-ACCEPT; was Draft) | [H26-toolchain-and-unicode.md](H26-toolchain-and-unicode.md) |
| H27 | Compilers as Cargo features | M0 (wk4) | E | Accepted (2026-10-22; review record complete, M2-P3-ACCEPT; was Draft (2026-10-21; feature-matrix built)) | [H27-compiler-features.md](H27-compiler-features.md) |
| H28 | Platform layer | M0 wk1-3 | U | Accepted (2026-10-22; review record complete, M2-P4-ACCEPT; was Draft (SP-JIT-mem passed on 4 CI runners on 2026-10-21: [data](../measurements/2026-10-21-sp-jit-mem.md))) | [H28-platform-layer.md](H28-platform-layer.md) |
| H29 | Profiling and deopt contract; position-independent metadata | Draft M0 month 3; Spike C + SP-Facts; frozen M1 exit | E | Draft (2026-10-22; position part backed by the Spike B offset-slot data, placements (a) and (b); reopened 2026-10-22: JIT frames on the VM stack (placement (b)); deopt-exit parts open for Spike C + SP-Facts) | [H29-deopt-contract.md](H29-deopt-contract.md) |
| H30 | IR fact model with provenance | Draft M0 month 3; frozen M1 exit | N | Draft (2026-10-22; FactSnapshot v0 names the 3 sources; confidence and VERIFIED open) | [H30-ir-fact-model.md](H30-ir-fact-model.md) |
| H31 | Fuse registry | API frozen M1 exit; built M1-M2 | N | Draft (2026-10-22; no registry built; proto_index is the as-built no-elements flag) | [H31-fuse-registry.md](H31-fuse-registry.md) |
| H32 | Guard-failure policy per mode | Frozen M1 exit; applied A3, A4 | N | Draft (2026-10-22; per-mode table and jitless budgets; applied A3, A4) | [H32-guard-failure-policy.md](H32-guard-failure-policy.md) |
| H33 | AOT feedback rule | Frozen M1 exit; built A3/A4 | N | Draft (2026-10-22; hybrid re-tier rule; built A3/A4, K-A4P decides hybrid) | [H33-aot-feedback-rule.md](H33-aot-feedback-rule.md) |
| H34 | One Rust backend with abstract relocations | ADR frozen M1 exit; built A2/M5 | N | Draft (2026-10-22; plan row and Spike B emitter evidence: no address, no relocation, tested for (b) on AArch64; x86-64 (b) built, x86-64 runs in the M3-P0 sandbox and CI; M3-P1 SP-UNIT built unit text with 0 relocations and filled H34-Q1 to Q3; Q4 and Q5 open) | [H34-relocation-kinds.md](H34-relocation-kinds.md) |
| H35 | AOT unit format | Spec v0 + header struct + golden test by M0 exit; writer/loader A2; frozen at A3 gate | N | Draft (header v0 and load check built 2026-10-21; H35-Q7 decided 2026-10-20; H35-Q8 filled 2026-10-22; H35-Q2 and H35-Q9 filled 2026-10-23, M3-P1 SP-UNIT; SP-Unit built, verdict by the orchestrator) | [H35-aot-unit-format.md](H35-aot-unit-format.md) |
| H36 | FP semantics in every backend | M0 rule (wk2); FP differential CI from M1 | N | Accepted (2026-10-22; review record complete, M2-P3-ACCEPT; was Draft (decided 2026-10-20; pow_js v0 and fp-check built 2026-10-21)) | [H36-fp-semantics.md](H36-fp-semantics.md) |
| H37 | TS hint side-table schema | v0 at A1h (by M4 exit); frozen at A4 start | N | Not started | `H37-hint-side-table.md` |

Counts on 2026-10-23 (M0 month 3 P1, from the Status line of each ADR file): 21 Draft, 2 Not started, 14 Accepted, 0 Frozen. M3-P1 moved no ADR to another Status (the B4 Accepts wait for M3-P2). Before M3-P1 this line said "Counts on 2026-10-22 (M0 month 2 P5)" with the same numbers. P5 accepted H23, H24, H25 and H26 (M2-P5-ACCEPT); H04 stays Draft until ABI 5 (M2-P5-PLAN-D23). After P5-DOCS-SYNC this line said "25 Draft, 2 Not started, 10 Accepted". P5-DOCS-SYNC added the Draft H34. After P4 this line said "24 Draft, 3 Not started, 10 Accepted". P4 accepted H1, H5, H6, H9, H10, H11 and H28 (M2-P4-ACCEPT). After P3 this line said "31 Draft, 3 Not started, 3 Accepted". P3 added the Drafts H29-H33 and accepted H3, H27 and H36 (M2-P3-ACCEPT). H6, H11, H35 and H15 carry decisions and move to Accepted when the §3.1 review record is complete. On 2026-10-21 (after P0) this line said "29 Draft, 8 Not started"; until then it said "11 Draft, 26 Not started", which did not include H15 or the five M0 wk3 drafts (H1, H2, H3, H5, H14).

## 6. When each ADR is due in M0

From [09-milestones §8](../plan/09-milestones.md#8-m0-first-month-week-by-week) and the M0 exit list.

```
M0 wk1   template; drafts H6 H7 H8 H9 H10 H11 H12 H26 H28; H35 spec skeleton (AOT senior)
M0 wk2   SP-Value (H6, H11), SP-VA (H7 + code range), SP-GC (H9, H10), SP-Dispatch (H15);
         H8 ShapeId rule, H36 `clippy.toml` bans + `xtask fp-check` stub (AOT senior)
M0 wk3   H1, H2 ABI + determinism CI, H3 header flags, H5 rule, H14 dispatch kinds + registry trait,
         H22 (D14), H23; SP-JIT-mem (H27, H28)
M0 wk4   H16 slot kinds as FactSnapshot v0; H27 (aot feature builds); H35 header struct + golden test;
         H2 and H14-H20 drafted
M0 m2-3  ADRs H4, H13 rules, H24, H25; drafts H29-H34; H20 frozen; Spike B + SP-Unit (H14, H34, H35; D21)
M0 exit  item 9: every "Fix by M0" row merged; item 10: A0 part 1 merged (H2, H3 fields, H5, H7, H8,
         H13 rules, H14, H27, H35 spec v0 with header struct and golden test, H36)
M1a      H17-H19 frozen
M1 exit  H2, H14, H16, H29-H34 frozen; no H-row change without an approved amendment
```
