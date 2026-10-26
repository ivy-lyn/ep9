# H26: Toolchain and Unicode

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P5-ACCEPT; review record complete). Was: Draft (2026-10-20) |
| Fix by | M0 wk1 |
| Freeze point | M1 exit |
| Origin | v1#24; plan status U (unchanged) |
| Crates | Whole root workspace (`rust-toolchain.toml`, `Cargo.lock`); `ep9_frontend` (oxc, unicode-id-start); `ep9_regexp` and `ep9_intl` (planned, 08 §2; not in `crates/` yet); `tools/ep9-typecheck` (own workspace; planned, A1h) |
| Owner (plan role) | Lead + platform lead |
| Validated by | Dependency guards ([08 §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks)); parse-only runner (M0 wk1 check) |
| Plan sources | [02 §6 H26](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [08 §5](../plan/08-repo.md#5-how-third_partytsrs-is-consumed), [08 §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks), [11 §3.1 D8](../plan/11-decisions.md#31-d1-d14-from-plan-v1) |

## Context

- Tail-call dispatch on nightly (`become`) is 0-3% faster than stable sibling calls. It is not stable and is not expected before 2027 [R06].
- tsrs pins `unicode-id-start =1.1.2` (Unicode 15.1). In a shared Cargo graph it drags oxc down to Unicode 15.1, and oxc then fails 16 test262 identifier tests [R02][R13].
- ep9 still needs tsrs, but only in separate workspaces: `tsrs_cli` for `ep9 check` and `tools/ep9-typecheck` for hints (N8).
- D8 (toolchain) took the plan default on 2026-10-20 ([decisions log](../decisions-log.md)).

## Decision

Stable 1.99.1; nightly only for opt-in `become`, fuzz, sanitizers. Unicode 17 everywhere (oxc via unicode-id-start 1.5.0, regress, ICU4X 2.3). ep9-typecheck uses tsrs's toolchain in its own workspace; its Unicode 15.1 pin never reaches the engine.

```
 root workspace (rust-toolchain.toml: 1.99.1) ---- engine graph: unicode-id-start >= 1.5.0 (Unicode 17)
   oxc_* =0.155.0, regress fork, icu 2.3                     no tsrs_core, ever
 third_party/tsrs (tsrs's own workspace + lock) ---- tsrs_cli; unicode-id-start =1.1.2 stays here
 tools/ep9-typecheck (own [workspace], tsrs lock) -- channel read from tsrs's rust-toolchain.toml (1.99.1)
 fuzz/ (own workspace) ----------------------------- nightly
```

## Rules

| Rule | Check | From |
|---|---|---|
| The root workspace builds with stable 1.99.1 | `rust-toolchain.toml` channel; CI on 4 targets | M0 wk1 |
| Nightly only behind opt-in: `become` expansion (H15), `fuzz/`, sanitizers | Feature/flag review; nightly never in default CI builds | M0 |
| `cargo tree -i unicode-id-start` in the root workspace shows only ≥ 1.5.0 | Dependency guard, every PR | M0 |
| `cargo tree -i tsrs_core` is empty for the root workspace | Dependency guard, every PR | M0 |
| `third_party` and `tools/ep9-typecheck` stay in `exclude` | Root `Cargo.toml` review; guard | M0 wk1 |
| ep9-typecheck's lock matches tsrs's lock, plus only `ep9-typecheck` and `ep9_hints` | Lock guard | A1h |

State on 2026-10-20 (skeleton report): `rust-toolchain.toml` sets `channel = "1.99.1"`. Both `cargo tree -i` guards exit 101 with "did not match any packages", because no crate uses oxc yet. In a scratch copy with all 7 oxc pins, unicode-id-start resolved to 1.5.0 with no tsrs.

State on 2026-10-22 (0c4c70e): oxc is in the graph; dep-guards checks (1) and (2) pass with `unicode-id-start` 1.5.0 and no `tsrs_core`. ICU4X 2.3 is in the graph only through oxc's diagnostics (`oxc_diagnostics` -> `textwrap` -> `icu_segmenter` 2.3.0); `regress` is not in the graph (no RegExp yet). Rust std 1.99.1 is Unicode 17.0.0 (`char::UNICODE_VERSION` = (17, 0, 0)), so std case mapping is at the H26 version. `third_party/tsrs/rust-toolchain.toml` is also 1.99.1.

## Consequences

- **Gains.** No nightly lock-in. The 16 Unicode 16/17 identifier tests pass. tsrs churn and its Unicode pin stay outside the engine.
- **Costs.** Stable sibling-call dispatch is not guaranteed TCO: the debug sibling-call build overflowed the stack [R06]. So H15 needs the disassembly check and a deep-loop test (RK15).
  - Two toolchain worlds to keep in step: the root pin and tsrs's pin. Today both are 1.99.1.
- **Follow-ups.** The pin-leak guard must treat "did not match any packages" (exit 101) as a pass for tsrs_core. For unicode-id-start that case ended when oxc entered the graph; dep-guards check (2) now reads the versions from `cargo metadata`.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | `become` tracking issue rust-lang/rust#112788 is OPEN (B-experimental); stable 1.99 rejects it; stabilization aimed at 2027. Sibling calls 0.53-0.60 ns vs become 0.52-0.59 ns | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R02] | tsrs's `unicode-id-start =1.1.2` pin forced oxc onto Unicode 15.1 in a combined build; oxc failed 16 Unicode 16/17 identifier tests | [02-tsrs-conformance.md](../research/02-tsrs-conformance.md) |
| [R13] | Linking tsrs and oxc_parser 0.155 in one binary resolves unicode-id-start to 1.1.2 instead of 1.5.0 | [13-tsrs-type-oracle.md](../research/13-tsrs-type-oracle.md) |

## Validation

- No spike in [09 §4](../plan/09-milestones.md#4-spikes) reopens H26.
- M0 wk1 check ([09 §8](../plan/09-milestones.md#8-m0-first-month-week-by-week)): parse runner gives 4,653/4,653 negatives and the 7 known positive misses on all 4 targets. A Unicode downgrade would show as 16 new positive misses.
- Shared command: `cargo run --release -p ep9_test262 -- parse` (exit 1 on regression).
- `bash scripts/sync-tsrs.sh` builds tsrs_cli with tsrs's own channel.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H26-Q1 | How the root toolchain is bumped, and whether a bump is an ADR amendment. The plan says only "same as tsrs" (08 §1) | Before the first bump | A bump PR of its own; kind **fill**; needs the 4-target CI and the parse runner green |
| H26-Q2 | What happens when tsrs moves to a newer channel than the root pin? `sync-tsrs.sh` reads tsrs's channel, so the tools follow tsrs; the engine does not | The first tsrs pin bump that moves tsrs's channel (the 745eb10 bump, 2026-10-26, kept 1.99.1 on both sides, so the case did not come up) | Engine stays; bump it in a separate PR |
| H26-Q3 | Which nightly date is pinned for `fuzz/` and sanitizers | M1 (fuzzing starts) | Pin one date in `fuzz/rust-toolchain.toml` |

## Reopens if

- A dependency guard fails and no graph change can fix it.
- D8 changes (nightly default) or D1 changes to a tsrs fork (11-decisions "Affects": D1 → H1, H26; D8 → H15, H26).
- `become` reaches stable Rust. Then H15's opt-in expansion can become default; review this row with H15.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6 and 08 §5, §7 (M0 wk1 list) | ADR agent |
| 2026-10-22 | clarify | Crates line: `ep9_regexp`, `ep9_intl` and `tools/ep9-typecheck` are planned and not in the tree yet. A "State on 2026-10-22" paragraph after the Rules (oxc in the graph, unicode-id-start 1.5.0, ICU4X only through oxc diagnostics, no regress, std Unicode 17.0.0); Follow-ups: the exit-101 case for unicode-id-start ended. No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | Review record row 2 names the P5 Codex run (slice accept4): 2 findings, A1 on H23 and A2 on H04, both fixed (clarify); no finding on this ADR. Text only; no check changes | orchestrator |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P5-ACCEPT). The §3.1 review record is complete: row 1 P4-ACCEPT-3, row 2 Codex (m2p4-docs run; P5 accept4 run), row 3 P5-ACCEPT-4 (with the P5 round-2 notes), row 4 data. The orchestrator did not write the rows it approves. No check changes | orchestrator |
| 2026-10-26 | clarify | H26-Q2 "Needed by": the tsrs pin bump f527e984 -> 745eb10a kept `channel = "1.99.1"` in tsrs's `rust-toolchain.toml`, the same as the root pin, so Q2 is now due at the first bump that moves tsrs's channel. The question and its proposed default are unchanged. Text only; no check changes. Review record rows 1-4 and Approval: pending | tsrs bump implementer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P4-ACCEPT-3 (did not write the ADR) | Decision = the 02 §6 H26 row, word for word. Checked at 0c4c70e: `rust-toolchain.toml` channel 1.99.1 (the tsrs pin is 1.99.1 too); `exclude` = third_party, fuzz, tools/ep9-typecheck, spikes (root `Cargo.toml`); the `become` feature of `ep9_vm` is opt-in; dep-guards checks (1) and (2) PASS; `cargo tree -i regress` matches no package; ICU4X 2.3 reaches the graph only through `oxc_diagnostics`; a `rustc +1.99.1` program prints `char::UNICODE_VERSION` = (17, 0, 0), "ß".to_uppercase() = "SS" and "ΑΣ".to_lowercase() = "ας" (final sigma). R1: the Crates line named `ep9_regexp` and `ep9_intl`, which are not in `crates/`, and `tools/ep9-typecheck`, which does not exist yet (fixed, clarify: planned). R2: the Rules state paragraph and the Follow-ups were from 2026-10-20, before oxc entered the graph (fixed, clarify: a dated state line). Open questions due in M0: none dated in M0; H26-Q1 ("before the first bump") becomes due only if the root toolchain is bumped in M0 (none planned). Change-log kinds: agree | branch m0m2/p4-accept-3 |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | The P4 docs run (slice m2p4-docs: the P4 edits of 12 ADRs, among them every B3 record and review fix in H04, H23, H24, H25 and H26; base 2e6940a = the P4 head 575569f without the slice): needs-attention, 1 finding, D1 on H09 and H10; no finding on H26. P5 run (slice accept4: the P5-ACCEPT-4 commits dffd611 and e4767c1, base bfd5cd8; Codex gpt-6.1-sol): needs-attention, 2 findings: A1 on H23 (host-function calls take no budget unit; fixed 6684988, clarify) and A2 on H04 (the spike-native match-loop fallback; fixed c4ca7de, clarify). No finding on this ADR. | Codex text outside the repo: scratchpad review/m2p4-docs.codex.txt (file list review/m2p4-docs.list); Codex text outside the repo: scratchpad m0m2/p5/codex-accept4.out |
| 3 Verifier | P5-ACCEPT-4 (wrote none of the rows it checks) | Checked at bfd5cd8. Rows 1 and 2 stay as written (row 2: no H26 finding in the m2p4-docs Codex text: true); no row was added after the Codex slice (375d953 and c39f782 did not touch H26). No new check (plan): `rust-toolchain.toml` (channel 1.99.1), the root `Cargo.toml`, `Cargo.lock`, `.github/scripts/dep-guards.sh` and `third_party/tsrs.commit` are unchanged since 0c4c70e (`git diff --stat 0c4c70e bfd5cd8` lists none of them). Open questions due in M0: none (Q1 is due only if the root toolchain is bumped in M0, and it was not; Q2 next tsrs pin bump; Q3 M1; the table is unchanged since 90db5ee). Part 4 again at bfd5cd8: `bash .github/scripts/dep-guards.sh` 29 PASS, 0 FAIL (check (1) no tsrs_core; check (2) unicode-id-start 1.5.0). Change-log kinds: agree | branch m0m2/p5-accept-4; local run, mac a64, bfd5cd8; logs outside the repo: scratchpad m0m2/p5/accept4/logs/depguards.log |
| 4 Data | from the "Validated by" line | `bash .github/scripts/dep-guards.sh`: 29 PASS, 0 FAIL (check (1) no tsrs_core; check (2) unicode-id-start 1.5.0). Parse runner: 4,653 / 4,653 negatives and 47,357 / 47,364 positives (the 7 known misses) at e719ffa (P3-ACCEPT-2, H01 row 4); the real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3), with the test262 parse step on 4 targets. Missing: the lock guard (A1h); not re-run here: `bash scripts/sync-tsrs.sh` | local run, mac a64, 0c4c70e; logs outside the repo: scratchpad m0m2/accept3/logs/depguards.log; scratchpad m0m2/accept2/logs/t262-parse.log |
| 1-4, Approval (tsrs-bump amendment: the 2026-10-26 H26-Q2 clarify row) | pending | Pending. Agents that did not write the clarify row fill rows 1-4 and the Approval row (README §3.1, §3.2). Data for row 4 from the bump run: tsrs 745eb10a `rust-toolchain.toml` channel 1.99.1 (root pin 1.99.1); `bash scripts/sync-tsrs.sh` exit 0; `bash .github/scripts/dep-guards.sh` 29 PASS, 0 FAIL | pending |
