# Upstream drafts

- Rules: [12-governance §5](../plan/12-governance.md#5-upstream-contact-oxc-and-other-public-projects) (decision UPSTREAM). **Only the user posts.** Agents write drafts here. They never create issues, PRs, comments, reactions or reviews on a public repository.
- One file per request: `<project>-<slug>.md`. Start from the template below.
- Status: draft -> ready (all 3 checks of 12-governance §5 pass) -> handed over (in a status report) -> posted (the user adds the link) or dropped.
- A draft is "ready" only when an agent that did not write it ran the repro again, on the pinned version and on upstream main. The three oxc drafts are ready since 2026-10-21 (checked by P0-OXC-VERIFY, M0 month 2).

| File | Project | Upstream ref | ep9 need (milestone, gate) | Local workaround | Status |
|---|---|---|---|---|---|
| [oxc-eval-parse-options.md](oxc-eval-parse-options.md) | oxc | #11793 (open since 2025-06-18) | Eval-context early errors; M1 exit; P1 | Script parse (no wrapper) that drops only the allowed nodes' errors, plus post-checks (`eval.rs` E1), then a local patch | ready (2026-10-21) |
| [oxc-depth-limit.md](oxc-depth-limit.md) | oxc | #21953, #24375 | Depth error instead of abort; M1 exit; P1 | Bracket pre-scan, sized stack, AST-depth pass (M0) | ready (2026-10-21) |
| [oxc-function-constructor.md](oxc-function-constructor.md) | oxc | none yet (new issue) | Parse Function() parameters and body apart; M1 exit | Three parses (`function_ctor.rs` F1): the parameter text and the body text each in a wrapper whose shape is checked, then the joined text through `compile_eval` | ready (2026-10-21) |
| [oxc-strict-eval-default-init.md](oxc-strict-eval-default-init.md) | oxc | none yet (new issue) | PRE-1: strict `eval`/`arguments` read inside a destructuring target; M1 front end, no gate | Post-filter after oxc_semantic (`strict_filter.rs`, M4-P2): drop the error when its IdentifierReference is not an assignment target | draft |
| [oxc-let-as-label.md](oxc-let-as-label.md) | oxc | none yet (new issue) | `let: 42` as a label in sloppy code (test262 staging/sm/syntax/let-as-label.js); no gate | None; the test stays an expected failure | draft |

## Shared repro program

All three oxc drafts use one small program on oxc `=0.155.0` (`oxc_allocator`, `oxc_parser`, `oxc_semantic`, `oxc_span`; Rust 1.99.1, release build, macOS arm64). It parses a string as a script with `parse_regular_expression = true`, then runs `SemanticBuilder::new().with_check_syntax_error(true)`. It prints the errors of the first stage that has any, or "accepted". The depth cases run on a thread with an 8 MiB stack, one child process per case. The verifier's version of the program, its driver script, the commands for the pinned and the main build, and the full output are in [oxc-repro-program.md](oxc-repro-program.md). Each draft quotes the part it needs.

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
