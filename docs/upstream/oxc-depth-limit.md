# parser: deep nesting aborts the process; a depth limit that returns a diagnostic

| Field | Value |
|---|---|
| Project, ref | oxc, [#21953](https://github.com/oxc-project/oxc/issues/21953) (statement bodies; open, opened 2026-04-29) and [#24375](https://github.com/oxc-project/oxc/issues/24375) (expressions, SIGSEGV in napi `parseSync`; open, opened 2026-07-11). Issue pages read 2026-10-21; no linked PR. The text below is a comment on #24375, which already links #21953 |
| Status | draft |
| ep9 need | Depth above the cap gives a RangeError, never an abort ([01-front-end §6](../plan/01-front-end.md#6-depth-guard-3-layers-unchanged), layer 1). Due M1 exit; gate P1 |
| Workaround | Bracket pre-scan sizes the parse stack; parse on a worker thread with a sized stack; iterative AST-depth pass after the parse (M0). A local patch only if these miss a form. Local oxc patches today: 0/3 |
| Verified by | not yet (needs an agent that did not write this draft) |

## Repro (pinned version)

oxc `=0.155.0`, Rust 1.99.1 release build, macOS arm64, 2026-10-21. Each case parses a generated script on a thread with an 8 MiB stack (`std::thread::Builder::stack_size(8 << 20)`), one child process per case. "parse only" stops after `Parser::parse`, so the abort is in the parser, not in semantic analysis.

| Form | Source for depth n | n = 1,000 | n = 10,000 | n = 50,000 |
|---|---|---|---|---|
| array | `[` × n, `]` × n | ok | abort | abort |
| paren | `(` × n, `0`, `)` × n | ok | abort | abort |
| conditional chain | `a ? b : ` × n, `0` | ok | ok | abort |
| for statements | `for (;;) ` × n, `;` | ok | ok | abort |
| blocks | `{` × n, `}` × n | ok | ok | abort |

- "abort" = the process dies with SIGABRT: "thread '<unknown>' has overflowed its stack". The same result with "parse only".
- Bisection, parse only: arrays and parens pass at 5,449 levels and abort at 5,497.
- `ParseOptions` in 0.155.0 has no depth option.

## Repro (upstream main)

Not run yet.

## Text to post

> Data for oxc_parser 0.155.0, Rust release build, macOS arm64. Each input is parsed on a thread with an 8 MiB stack. "abort" means the process dies with "has overflowed its stack" inside `Parser::parse` (no semantic pass).
>
> | Form | n = 1,000 | n = 10,000 | n = 50,000 |
> |---|---|---|---|
> | `[`×n `]`×n | ok | abort | abort |
> | `(`×n `0` `)`×n | ok | abort | abort |
> | `a ? b : `×n `0` | ok | ok | abort |
> | `for (;;) `×n `;` | ok | ok | abort |
> | `{`×n `}`×n | ok | ok | abort |
>
> Nested arrays and parens abort at about 5,500 levels with 8 MiB. So expression nesting (this issue) hits the limit before statement nesting (#21953).
>
> Raising the stack size helps, but a library user cannot know how much stack an input needs before parsing it. A request that fits the comments in #21953 (no `stacker`, no rewrite to loops):
>
> - A `ParseOptions` field such as `max_nesting_depth: Option<u32>`. One counter, incremented on entry to the recursive productions (bracketed expressions, array and object literals, conditional and assignment expressions, unary chains, `**`, template substitutions, arrow and function bodies, classes, statement bodies), decremented on exit.
> - Above the limit, the parser returns a diagnostic ("nesting too deep") and stops.
> - The default can be `None`, so nothing changes for current users. A user who knows its stack size picks a limit that fits it.
