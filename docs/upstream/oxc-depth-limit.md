# parser: deep nesting aborts the process; a depth limit that returns a diagnostic

| Field | Value |
|---|---|
| Project, ref | oxc, [#21953](https://github.com/oxc-project/oxc/issues/21953) (statement bodies; open, opened 2026-04-29) and [#24375](https://github.com/oxc-project/oxc/issues/24375) (expressions, SIGSEGV in napi `parseSync`; open, opened 2026-07-11). Issue pages read 2026-10-21. One PR existed: [#24577](https://github.com/oxc-project/oxc/pull/24577) "fix(parser): guard deeply nested expressions" (a fixed limit of 256 levels, always on). It was closed without a merge on 2026-10-17; the maintainer said "not planned", because the error is rare and seen only in synthetic workloads. Earlier, on #24375 (2026-07-16), a maintainer suggested `RUST_MIN_STACK` as the workaround. The text below is a comment on #24375, which already links #21953 |
| Status | ready (2026-10-21) |
| ep9 need | Depth above the cap gives a RangeError, never an abort ([01-front-end §6](../plan/01-front-end.md#6-depth-guard-3-layers-unchanged), layer 1). Due M1 exit; gate P1 |
| Workaround | Bracket pre-scan sizes the parse stack; parse on a worker thread with a sized stack; iterative AST-depth pass after the parse (M0). A local patch only if these miss a form. Local oxc patches today: 0/3 |
| Verified by | P0-OXC-VERIFY (M0 month 2), 2026-10-21. Ran all 15 cells in full and parse-only mode again on oxc `=0.155.0` (release commit `73e6b564`) and on oxc main `f3236009` (2026-10-21): the same table on both. Bisection on both (parse only): arrays pass at 5,465 and abort at 5,466; parens pass at 5,464 and abort at 5,465 (inside the 5,449-5,497 range below). Program, commands and full output: [oxc-repro-program.md](oxc-repro-program.md) |

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

oxc main `f32360093c2ee18096fa14e0f48c48d75ca5a4a4` (2026-10-21T05:21:14Z), shallow clone, path dependencies, Rust 1.99.1 release build, macOS arm64, run 2026-10-21 by P0-OXC-VERIFY. Same program, 8 MiB thread, one child process per case.

| Form | n = 1,000 | n = 10,000 | n = 50,000 |
|---|---|---|---|
| array | ok | abort | abort |
| paren | ok | abort | abort |
| conditional chain | ok | ok | abort |
| for statements | ok | ok | abort |
| blocks | ok | ok | abort |

- Each cell gives the same result with semantic ("full") and with "parse only". Every abort is SIGABRT (signal 6) with "thread '<unknown>' has overflowed its stack".
- Bisection, parse only: arrays last ok 5,465, first abort 5,466; parens last ok 5,464, first abort 5,465. The pinned 0.155.0 build gives the same numbers.
- `ParseOptions` on main has the same 5 fields as 0.155.0, and `oxc_parser/src` has no depth limit (no match for `nesting_depth` or `max_depth`).

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
>
> This differs from #24577 (an always-on limit of 256, closed as not planned): it is off by default, so tools that parse trusted code see no change. It is for embedders that parse untrusted input, such as a JS engine or a service that parses user code. For them a deep input is not only a synthetic case: anyone can send one, and `RUST_MIN_STACK` only moves the depth at which the process dies.
