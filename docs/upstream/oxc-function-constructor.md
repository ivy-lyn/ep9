# parser: parse `Function()` parameters and body on their own (CreateDynamicFunction)

| Field | Value |
|---|---|
| Project, ref | oxc, new issue (no existing issue found in the plan research; search again before posting). Searched again 2026-10-21 by P0-OXC-VERIFY with the GitHub issue search on oxc-project/oxc: `CreateDynamicFunction` 0 hits; `"Function constructor"`, `"new Function" parser`, `"parse_formal_parameters"` and `"FormalParameters" parse string` list no issue or PR about this request |
| Status | ready (2026-10-21) |
| ep9 need | `compile_function_ctor(params, body, kind)` ([01-front-end §5](../plan/01-front-end.md#5-front-end-boundary-and-compiledunit-h1-h2)) must throw SyntaxError when the parameters or the body do not parse on their own. Due M1 exit ([01-front-end §7](../plan/01-front-end.md#7-oxc-upstream-track-local-patches-p-gates-unchanged)) |
| Workaround | Since M0 month 2 P3 (P3-FNCTOR), ep9 uses a three-parse check in `crates/ep9_frontend/src/function_ctor.rs` (its decisions F1 and F2): (a) the parameter text alone inside the wrapper `(function anonymous(` + P + `\n) {\n})` and (b) the body text alone inside `(function anonymous(\n) {\n` + body + `\n})`, each parsed and checked for its shape (one parenthesized function expression; the part ends at the wrapper's own `)` or `}`), then (c) the joined text through `compile_eval` for every early error. Both calls in the table below throw SyntaxError (test `function_ctor::tests::the_upstream_cases_are_syntax_errors`; node v24.22 oracle `crates/ep9_frontend/tests/function_ctor_cases.txt`). The upstream entry points would replace (a) and (b). Before P3 the plan named Nova's single-script check, which misses some cases (01-front-end §7). Local oxc patches today: 0/3 |
| Verified by | P0-OXC-VERIFY (M0 month 2), 2026-10-21. Ran both joined sources again on oxc `=0.155.0` (release commit `73e6b564`) and on oxc main `f3236009` (2026-10-21): both "accepted" on both. node v24.22.0 gives the two SyntaxErrors in the table. Program, commands and full output: [oxc-repro-program.md](oxc-repro-program.md) |

## Repro (pinned version)

oxc `=0.155.0`, Rust 1.99.1, macOS arm64, 2026-10-21. The program builds the source text that CreateDynamicFunction builds, `(function anonymous(` + P + `\n) {\n` + body + `\n})`, and parses it as a script (parser + `SemanticBuilder::with_check_syntax_error(true)`).

| Call | Joined source | oxc 0.155.0 on the joined source | Spec | node 24.22 |
|---|---|---|---|---|
| `new Function("/*", "*/){")` | `(function anonymous(/*\n) {\n*/){\n})` | accepted | SyntaxError: P alone is not FormalParameters | SyntaxError "Unexpected end of arg string" |
| `new Function("", "}); (function(){")` | `(function anonymous(\n) {\n}); (function(){\n})` | accepted | SyntaxError: body alone is not FunctionBody | SyntaxError "Single function literal required" |

- So one parse of the joined text is not enough. ECMA-262 CreateDynamicFunction parses P with the FormalParameters goal and the body with the FunctionBody goal (with the Yield and Await parameters of the kind), each on its own, and throws SyntaxError if either fails. Then it parses the joined text.
- oxc 0.155.0 has no public entry point that parses a string as FormalParameters or as FunctionBody. `Parser` has only `parse` and `parse_expression` (`oxc_parser-0.155.0/src/lib.rs`).

## Repro (upstream main)

oxc main `f32360093c2ee18096fa14e0f48c48d75ca5a4a4` (2026-10-21T05:21:14Z), shallow clone, path dependencies, Rust 1.99.1 release build, macOS arm64, run 2026-10-21 by P0-OXC-VERIFY. Same program (`oxc-repro check '<joined source>'`).

| Call | oxc main on the joined source |
|---|---|
| `new Function("/*", "*/){")` | accepted |
| `new Function("", "}); (function(){")` | accepted |

- The public entry points of `Parser` on main are still `parse` and `parse_expression` (`crates/oxc_parser/src/lib.rs`).
- The parser has the two parts inside: `parse_function_body` and `parse_formal_parameters` are `pub(crate)` in `src/js/function.rs`, on main and in 0.155.0. So most of the grammar code exists. The request is mainly a public entry point that sets the Yield and Await context and reports left-over tokens.

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
