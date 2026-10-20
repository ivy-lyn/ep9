# parser/semantic: eval-context options (new.target, super, super(), private names, field initializer, strict)

| Field | Value |
|---|---|
| Project, ref | oxc, [#11793](https://github.com/oxc-project/oxc/issues/11793) "parser/semantic: Support eval string parsing". Open; opened 2025-06-18; no linked PR (issue page read 2026-10-21). The text below is a comment on it, not a new issue |
| Status | draft |
| ep9 need | `compile_eval(src, EvalContext)` must give the right early errors for direct eval ([01-front-end §5, §7](../plan/01-front-end.md#7-oxc-upstream-track-local-patches-p-gates-unchanged)). Due M1 exit; gate P1 |
| Workaround | Nova-style wrapper parse plus post-checks, then a local patch in `third_party/patches/oxc/`. Local oxc patches today: 0/3. After a patch for this: 1/3 |
| Verified by | not yet (needs an agent that did not write this draft) |

## Repro (pinned version)

oxc `=0.155.0`, Rust 1.99.1, macOS arm64, 2026-10-21. The program parses each string as a script, then runs semantic syntax checks:

```rust
let ret = Parser::new(&alloc, src, SourceType::script())
    .with_options(ParseOptions { parse_regular_expression: true, ..ParseOptions::default() })
    .parse();
// if the parser has no errors:
let sem = SemanticBuilder::new().with_check_syntax_error(true).build(&ret.program);
```

| # | Eval text | Context of the direct eval call | oxc 0.155.0 | Spec (node 24.22 agrees) |
|---|---|---|---|---|
| 1 | `new.target` | inside a function | parser: "Unexpected new.target expression" | valid |
| 2 | `super.x` | inside a method | semantic: "'super' can only be referenced in members of derived classes or object literal expressions." | valid |
| 3 | `super()` | inside a derived constructor | semantic: "Super calls are not permitted outside constructors or in nested functions inside constructors." | valid |
| 4 | `this.#x` | inside a class that declares `#x` | semantic: "Private identifier '#x' is not allowed outside class bodies" | valid |
| 5 | `arguments` | inside a class field initializer | accepted | SyntaxError |
| 6 | `with (o) {}` | from strict code | accepted | SyntaxError |

- `ParseOptions` in 0.155.0 has `parse_regular_expression`, `allow_return_outside_function`, `preserve_parens`, `allow_v8_intrinsics` and `enable_ident_hashes`. None of them sets an eval context.
- Case 6 has a workaround outside oxc: oxc_semantic makes the top-level scope strict when the program has a `"use strict"` directive, so an embedder can add one. Cases 1-5 have none.

## Repro (upstream main)

Not run yet.

## Text to post

> A data point for this issue, from oxc 0.155.0 (parser + `SemanticBuilder::with_check_syntax_error(true)`, source type script):
>
> | Eval text | Caller context | oxc | Spec |
> |---|---|---|---|
> | `new.target` | function | error "Unexpected new.target expression" | valid |
> | `super.x` | method | error "'super' can only be referenced in members of derived classes or object literal expressions." | valid |
> | `super()` | derived constructor | error "Super calls are not permitted outside constructors or in nested functions inside constructors." | valid |
> | `this.#x` | class that declares `#x` | error "Private identifier '#x' is not allowed outside class bodies" | valid |
> | `arguments` | class field initializer | accepted | SyntaxError |
>
> These follow from PerformEval and the eval-specific early errors: the eval text is a Script, but the caller's context decides whether `new.target`, `super.x`, `super()`, `#x` and `arguments` are allowed.
>
> Today an embedder has two weak options. It can drop these diagnostics after the parse, but that depends on the message text, and it cannot add the missing error of row 5. Or it can parse the text inside a wrapper (for example a method body), but then the wrapper accepts text that eval must reject, such as `return 1`.
>
> The Babel-style options suggested above would cover this. Concretely, for each direct eval call:
>
> - `allow_new_target_outside_function` (row 1)
> - `allow_super_outside_method` (row 2)
> - `allow_super_call_outside_constructor` (row 3)
> - a set of private names that are in scope (row 4)
> - "in class field initializer", which makes `arguments` an early error (row 5)
> - a strict flag for the eval text
>
> The private-name set could be the last step, if it is the hard part.
