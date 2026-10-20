# H26: Toolchain and Unicode

| Field | Value |
|---|---|
| Status | Draft (2026-10-20) |
| Fix by | M0 wk1 |
| Freeze point | M1 exit |
| Origin | v1#24; plan status U (unchanged) |
| Crates | Whole root workspace (`rust-toolchain.toml`, `Cargo.lock`); `ep9_frontend` (oxc, unicode-id-start); `ep9_regexp`; `ep9_intl`; `tools/ep9-typecheck` (own workspace) |
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

## Consequences

- **Gains.** No nightly lock-in. The 16 Unicode 16/17 identifier tests pass. tsrs churn and its Unicode pin stay outside the engine.
- **Costs.** Stable sibling-call dispatch is not guaranteed TCO: the debug sibling-call build overflowed the stack [R06]. So H15 needs the disassembly check and a deep-loop test (RK15).
  - Two toolchain worlds to keep in step: the root pin and tsrs's pin. Today both are 1.99.1.
- **Follow-ups.** The pin-leak guard must treat "did not match any packages" (exit 101) as a pass for tsrs_core, and for unicode-id-start until oxc enters the graph (skeleton issue note).

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
| H26-Q2 | What happens when tsrs moves to a newer channel than the root pin? `sync-tsrs.sh` reads tsrs's channel, so the tools follow tsrs; the engine does not | Next tsrs pin bump | Engine stays; bump it in a separate PR |
| H26-Q3 | Which nightly date is pinned for `fuzz/` and sanitizers | M1 (fuzzing starts) | Pin one date in `fuzz/rust-toolchain.toml` |

## Reopens if

- A dependency guard fails and no graph change can fix it.
- D8 changes (nightly default) or D1 changes to a tsrs fork (11-decisions "Affects": D1 → H1, H26; D8 → H15, H26).
- `become` reaches stable Rust. Then H15's opt-in expansion can become default; review this row with H15.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6 and 08 §5, §7 (M0 wk1 list) | ADR agent |
