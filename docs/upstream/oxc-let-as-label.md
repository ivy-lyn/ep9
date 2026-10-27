# parser: `let: 42` (a label named `let`) is rejected in sloppy script code

| Field | Value |
|---|---|
| Project, ref | oxc, new issue (not searched yet; search the oxc-project/oxc issues for `let` label / LabelledStatement before posting) |
| Status | draft |
| ep9 need | test262 `test/staging/sm/syntax/let-as-label.js` (sloppy `Function("let: 42")` and `eval("let: 42")` must parse). Not on an M0 or M1 gate; the file stays an expected failure in `expectations/test262.<triple>.jit.txt` |
| Workaround | None. No code change in M0 (P4 integration fixer). Local oxc patches today: 0/3 |
| Verified by | (none yet; an agent that did not write this draft runs the repro on oxc `=0.155.0` and on oxc main before the status moves to ready) |

## Repro (pinned version)

oxc `=0.155.0` (`Cargo.toml:31`), through the ep9 CLI (`crates/ep9_cli`, debug build, Rust 1.99.1, macOS arm64), ep9 commit 02c0be7, 2026-10-22. Each line is one script file, run as `ep9 <file>`. node v24.22.1 is the oracle.

| Source (script, sloppy) | ep9 (oxc 0.155.0) | node 24.22.1 | Spec |
|---|---|---|---|
| `let: 42` | `a.js:1:4: SyntaxError: Unexpected token` | runs, exit 0 | valid: a LabelledStatement with the label `let` |
| `let`, newline, `: 42` | `n.js:2:1: SyntaxError: Unexpected token` | runs, exit 0 | valid |
| `let: 42` | runs, exit 0 | (not run) | valid |
| `foo: 42; print("label ok");` | prints `label ok` | - | valid |
| `eval("let: 42"); print("eval ok");` | `c.js:1:1: Uncaught SyntaxError: Unexpected token` | - | valid (sloppy eval code) |
| `Function("let: 42"); print("fn ok");` | `d.js:1:1: Uncaught SyntaxError: Unexpected token` | - | valid (sloppy function code) |
| `"use strict"; let: 42` | `s.js:1:18: SyntaxError: Unexpected token` | `SyntaxError: Unexpected strict mode reserved word` | SyntaxError (strict code) |

- ECMA-262 §14.13 LabelledStatement is `LabelIdentifier : LabelledItem`. `let` is not a ReservedWord, so it is a valid LabelIdentifier. The only early error for it is §13.1.1 (Identifier : IdentifierName but not ReservedWord): a SyntaxError if the code is strict and the name is `let` (with `implements`, `interface`, `package`, `private`, `protected`, `public`, `static`, `yield`). So `let: 42` is valid in sloppy code and an error in strict code.
- Where oxc rejects it (`oxc_parser-0.155.0/src/js/declaration.rs`, `parse_let`): an unescaped `let` peeks at the next token. A `:` matches none of the cases (assignment or binary operator; `.`, `?.`, `(`; `;`; a single-statement context), so the last branch bumps `let` and parses a `let` declaration, which fails at the `:` (column 4). The escaped form `let` does not take `parse_let` (`src/js/statement.rs:174`, `Kind::Let if !self.cur_token().escaped()`), so it parses as an identifier and then as a label.
- The fix looks small: in `parse_let`, when the peeked token is `Kind::Colon`, parse the identifier as a label (the path that `let` already takes). Strict code must still report the §13.1.1 error.

## Repro (upstream main)

Not run yet. The verifier builds the same source texts against oxc main and records the commit and the result here.

## Text to post

> **Parser: `let` as a label (`let: 42`) is rejected in sloppy mode**
>
> In sloppy (non-strict) script code, `let: 42;` is a valid LabelledStatement: `let` is not a reserved word, so it is a valid LabelIdentifier (ECMA-262 §14.13). The only early error is §13.1.1, which applies in strict mode code only. Node, for example, runs `let: 42` without error and reports "Unexpected strict mode reserved word" for `"use strict"; let: 42`.
>
> oxc_parser 0.155.0 reports "Unexpected token" at the `:` for:
>
> ```js
> let: 42
> ```
>
> and also for `let` followed by a newline and `: 42`. The escaped form `let: 42` is accepted.
>
> Cause: `parse_let` (`crates/oxc_parser/src/js/declaration.rs`) treats a `let` that is followed by `:` as the start of a `let` declaration. Handling `Kind::Colon` there as a label (the path the escaped form already takes) would fix it; strict code must keep the §13.1.1 error.
>
> Test: test262 `test/staging/sm/syntax/let-as-label.js` (flag noStrict) calls `Function("let: 42")` and `eval("let: 42")` and expects both to parse.
