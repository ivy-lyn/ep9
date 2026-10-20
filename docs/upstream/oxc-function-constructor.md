# parser: parse `Function()` parameters and body on their own (CreateDynamicFunction)

| Field | Value |
|---|---|
| Project, ref | oxc, new issue (no existing issue found in the plan research; search again before posting) |
| Status | draft |
| ep9 need | `compile_function_ctor(params, body, kind)` ([01-front-end §5](../plan/01-front-end.md#5-front-end-boundary-and-compiledunit-h1-h2)) must throw SyntaxError when the parameters or the body do not parse on their own. Due M1 exit ([01-front-end §7](../plan/01-front-end.md#7-oxc-upstream-track-local-patches-p-gates-unchanged)) |
| Workaround | Nova's single-script check, which misses some cases (01-front-end §7). Local oxc patches today: 0/3 |
| Verified by | not yet (needs an agent that did not write this draft) |

## Repro (pinned version)

oxc `=0.155.0`, Rust 1.99.1, macOS arm64, 2026-10-21. The program builds the source text that CreateDynamicFunction builds, `(function anonymous(` + P + `\n) {\n` + body + `\n})`, and parses it as a script (parser + `SemanticBuilder::with_check_syntax_error(true)`).

| Call | Joined source | oxc 0.155.0 on the joined source | Spec | node 24.22 |
|---|---|---|---|---|
| `new Function("/*", "*/){")` | `(function anonymous(/*\n) {\n*/){\n})` | accepted | SyntaxError: P alone is not FormalParameters | SyntaxError "Unexpected end of arg string" |
| `new Function("", "}); (function(){")` | `(function anonymous(\n) {\n}); (function(){\n})` | accepted | SyntaxError: body alone is not FunctionBody | SyntaxError "Single function literal required" |

- So one parse of the joined text is not enough. ECMA-262 CreateDynamicFunction parses P with the FormalParameters goal and the body with the FunctionBody goal (with the Yield and Await parameters of the kind), each on its own, and throws SyntaxError if either fails. Then it parses the joined text.
- oxc 0.155.0 has no public entry point that parses a string as FormalParameters or as FunctionBody. `Parser` has only `parse` and `parse_expression` (`oxc_parser-0.155.0/src/lib.rs`).

## Repro (upstream main)

Not run yet.

## Text to post

> **Feature request: parse a string as FormalParameters or as FunctionBody**
>
> `new Function(p, body)` (and the generator, async and async generator forms) needs two parses that oxc does not expose. ECMA-262 CreateDynamicFunction parses the parameter string with the goal `FormalParameters[~Yield, ~Await]` (or the `+Yield` / `+Await` forms for the other kinds) and the body string with `FunctionBody` (or `GeneratorBody`, `AsyncFunctionBody`, `AsyncGeneratorBody`), each on its own. Only then does it parse the joined text `(function anonymous(p\n) {\nbody\n})`.
>
> Parsing only the joined text accepts inputs that must throw. With oxc_parser 0.155.0 (plus semantic syntax checks), both of these joined sources are accepted:
>
> ```js
> new Function("/*", "*/){")             // (function anonymous(/*\n) {\n*/){\n})
> new Function("", "}); (function(){")   // (function anonymous(\n) {\n}); (function(){\n})
> ```
>
> V8 throws SyntaxError for both ("Unexpected end of arg string", "Single function literal required").
>
> A possible API: two entry points on `Parser`, for example `parse_formal_parameters(kind)` and `parse_function_body(kind)`, where `kind` gives the Yield and Await parameters. Each returns its node and diagnostics, and reports an error when tokens are left over. An alternative is one entry point that takes the two strings and does all the CreateDynamicFunction parse steps.
