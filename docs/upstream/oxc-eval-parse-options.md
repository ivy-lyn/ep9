# parser/semantic: eval-context options (new.target, super, super(), private names, field initializer, strict)

| Field | Value |
|---|---|
| Project, ref | oxc, [#11793](https://github.com/oxc-project/oxc/issues/11793) "parser/semantic: Support eval string parsing". Open; opened 2025-06-18; no linked PR (issue page read 2026-10-21). The text below is a comment on it, not a new issue |
| Status | ready (2026-10-21) |
| ep9 need | `compile_eval(src, EvalContext)` must give the right early errors for direct eval ([01-front-end §5, §7](../plan/01-front-end.md#7-oxc-upstream-track-local-patches-p-gates-unchanged)). Due M1 exit; gate P1 |
| Workaround | Nova-style wrapper parse plus post-checks, then a local patch in `third_party/patches/oxc/`. Local oxc patches today: 0/3. After a patch for this: 1/3 |
| Verified by | P0-OXC-VERIFY (M0 month 2), 2026-10-21. Ran rows 1-6 again on oxc `=0.155.0` (release commit `73e6b564`) and on oxc main `f3236009` (2026-10-21): the same output as the table below on both. node v24.22.0 agrees with the Spec column. #11793 is still open (2 comments, last update 2025-06-20; read 2026-10-21). Program, commands and full output: [oxc-repro-program.md](oxc-repro-program.md) |

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

### AST after the error (rows 1-4)

Checked by P0-OXC-VERIFY. Question: when oxc 0.155.0 reports the row 1-4 error, does it still build the whole AST? This decides whether ep9 can drop the error and compile the AST it got.

Answer: yes, for all four rows, on 0.155.0 and on main.

- Command: `oxc-repro ast '<text>'` ([oxc-repro-program.md](oxc-repro-program.md)). It parses, runs semantic, then counts the row's node with a `Visit` pass and prints the codegen of the AST.
- Row 1 is a recoverable parser error: `parse_new_expression` calls `self.error(new_target_outside_function)` and still returns `Expression::new_new_target` (oxc_parser-0.155.0 `src/js/expression.rs` line 1011; the same line on main). `fatal_error` stays false.
- Rows 2-4 have no parser error. The error comes from semantic, which does not change the AST.

| # | Text | Parser errors | Semantic errors | `fatal_error` | Statements | Node in the AST | Codegen of the AST |
|---|---|---|---|---|---|---|---|
| 1 | `new.target` | 1 | 0 (semantic run anyway) | false | 1 | 1 `NewTarget` | `new.target;` |
| 2 | `super.x` | 0 | 1 | false | 1 | 1 `StaticMemberExpression` with object `Super` | `super.x;` |
| 3 | `super()` | 0 | 1 | false | 1 | 1 `CallExpression` with callee `Super` | `super();` |
| 4 | `this.#x` | 0 | 1 | false | 1 | 1 `PrivateFieldExpression` | `this.#x;` |

Raw output (0.155.0; main prints the same):

```
new.target   semantic_errors=0 parser_errors=1 fatal_error=false statements=1 new.target=1 super.x=0 super()=0 #field=0 codegen="new.target;"
super.x      semantic_errors=1 parser_errors=0 fatal_error=false statements=1 new.target=0 super.x=1 super()=0 #field=0 codegen="super.x;"
super()      semantic_errors=1 parser_errors=0 fatal_error=false statements=1 new.target=0 super.x=0 super()=1 #field=0 codegen="super();"
this.#x      semantic_errors=1 parser_errors=0 fatal_error=false statements=1 new.target=0 super.x=0 super()=0 #field=1 codegen="this.#x;"
```

Several uses: the text `new.target + 1; f(new.target)` gives 2 parser errors, one per `new.target` span, `fatal_error=false`, and an AST with both nodes (codegen `new.target + 1;\nf(new.target);`). So a drop matched by span removes each of them and keeps any other parser error.

## Repro (upstream main)

oxc main `f32360093c2ee18096fa14e0f48c48d75ca5a4a4` (2026-10-21T05:21:14Z), shallow clone, path dependencies, Rust 1.99.1 release build, macOS arm64, run 2026-10-21 by P0-OXC-VERIFY. Same program and command as the pinned run.

| # | Eval text | oxc main |
|---|---|---|
| 1 | `new.target` | parser: "Unexpected new.target expression" |
| 2 | `super.x` | semantic: "'super' can only be referenced in members of derived classes or object literal expressions." |
| 3 | `super()` | semantic: "Super calls are not permitted outside constructors or in nested functions inside constructors." |
| 4 | `this.#x` | semantic: "Private identifier '#x' is not allowed outside class bodies" |
| 5 | `arguments` | accepted |
| 6 | `with (o) {}` | accepted |
| - | `"use strict"; with (o) {}` (the row 6 workaround) | semantic: "'with' statements are not allowed" |
| - | `return 1` (what a wrapper parse would wrongly accept) | parser: "A 'return' statement can only be used within a function body." |

- `ParseOptions` on main has the same 5 fields as 0.155.0 (the struct text is identical). No eval-context option yet.

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
