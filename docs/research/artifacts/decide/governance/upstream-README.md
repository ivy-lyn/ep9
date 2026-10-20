# Upstream drafts

- Rules: [governance.md §5](../../../../plan/12-governance.md#5-upstream-contact-oxc-and-other-public-projects). Only the user posts. Agents write drafts here and never post.
- One file per request: `<project>-<slug>.md`. Start from the template below.
- The table lists every draft. Status: draft -> ready (all 3 checks pass) -> handed over (in a status report) -> posted (the user adds the link) or dropped.

| File | Project | Upstream ref | ep9 need (milestone, gate) | Local workaround | Status |
|---|---|---|---|---|---|
| `oxc-eval-parse-options.md` (to write) | oxc | #11793 (open since 2025-06-18) | Eval-context early errors; M1 exit; P1 | Nova-style wrapper parse + post-checks, then a local patch | not started |
| `oxc-depth-limit.md` (to write) | oxc | #21953, #24375 | Depth error instead of abort; M1 exit; P1 | Bracket pre-scan, sized stack, AST-depth pass (M0) | not started |
| `oxc-function-constructor.md` (to write) | oxc | none yet | Parse Function() parameters and body apart; M1 exit | Nova's single-script check | not started |

## Template

```md
# <one-line title, as it would appear upstream>

| Field | Value |
|---|---|
| Project, ref | oxc, #NNNN or "new issue" |
| Status | draft / ready / handed over / posted (<link>) / dropped |
| ep9 need | <milestone, gate>, due <date> |
| Workaround | <what ep9 does until upstream has it>; local patch count after this: n/3 |
| Verified by | <agent role that did not write the draft>, <date> |

## Repro (pinned version)
<code + command + actual vs expected, on oxc =0.155.0>

## Repro (upstream main, <date>, <commit>)
<same, result>

## Text to post
<standalone text: no ep9 internals, no private links>
```
