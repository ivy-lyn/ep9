# Hnn: <decision name from 02-architecture §6>

<!--
How to use this template:
- Copy it to docs/adr/Hnn-<slug>.md. Use the planned file name from README.md §5.
- Write in Simplified Technical English: short sentences, active voice, tables and diagrams.
- Copy the decision from the plan. Do not add design. If the plan leaves a gap, list it under
  "Open questions" and keep it out of the Decision section until an amendment resolves it.
- Keep the file between 60 and 150 lines.
- Update the status in README.md §5 in the same PR.
-->

| Field | Value |
|---|---|
| Status | Draft / Accepted / Frozen (date) |
| Fix by | Copied from 02-architecture §6 |
| Freeze point | From README.md §2 |
| Origin | v1#n and/or R16-Dn; plan status U, E or N |
| Crates | The crates this row governs (the ADR lint reads this line) |
| Owner (plan role) | From 09-milestones §8 or §4 |
| Validated by | SP-* spike, gate or CI check |
| Plan sources | Links to the plan sections that hold the detail |

## Context

- What problem the row solves. Which goals (G*) and non-goals (N*) it serves.
- Which other H rows and D decisions it depends on, and which depend on it.

## Decision

- The choice, copied from the plan row and its owning plan section.
- Use the plan's words. Link to the section.

## Rules

| Rule | Check | From |
|---|---|---|
| A testable statement | The test, lint, assert or gate that enforces it | When the check starts |

## Consequences

- **Gains.** What the choice buys, with numbers from the evidence.
- **Costs.** What it costs or rules out.
- **Follow-ups.** Work that other rows or milestones must do because of it.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [Rnn] | The finding, one line | [../research/nn-name.md](../research/README.md) |

## Validation

- The spike from 09-milestones §4 that checks the row, its gate and its milestone.
- CI checks from 10-quality §5 or 08-repo §7.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| Hnn-Q1 | A gap the plan leaves | Milestone or week | Optional; not part of the decision |

## Reopens if

- The named spike gate fails (09-milestones §4).
- A user decision changes (D*), as its "Affects" column in 11-decisions says.
- Any other measured trigger the plan names.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| YYYY-MM-DD | draft / clarify / fill / rewrite / reopen | What changed and why | Role |
