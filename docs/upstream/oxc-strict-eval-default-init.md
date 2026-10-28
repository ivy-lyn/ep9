# Strict mode: `eval`/`arguments` read inside a destructuring assignment is reported as an assignment

| Field | Value |
|---|---|
| Project, ref | oxc, new issue |
| Status | draft |
| ep9 need | PRE-1 (M3-P5-OPEN carry item); M1 front-end correctness, no gate. Due: none (the workaround is in place) |
| Workaround | `crates/ep9_frontend/src/strict_filter.rs` (M4-P2): after oxc_semantic, drop a "Cannot assign to 'eval'/'arguments' in strict mode" diagnostic whose label is an IdentifierReference that is not itself an assignment target, except an `arguments` read in a class field initializer or static block (then the class error is reported, see Cause). Used by `semantic` (lib.rs) and `compile_eval` (eval.rs). No oxc patch: local patch count stays 0/3 |
| Verified by | not yet (a draft is "ready" only after an agent that did not write it runs the repro again, README) |

## Cause

`oxc_semantic` `src/checker/javascript.rs`, `check_identifier_reference` (0.155.0 and main `def47b8`, the ancestor walk at lines 296-329): for an `eval` or `arguments` IdentifierReference in strict code, the checker walks all ancestors and reports at the first `ArrayAssignmentTarget`, `ObjectAssignmentTarget`, `AssignmentTargetPropertyIdentifier` or `UpdateExpression`. The walk stops only at a member expression. So a reference that is only read, in a default value, a computed key or a nested function inside a destructuring target, gets the error.

The walk also `return`s right after it reports, so the class check below it in the same function (§15.7.1: `arguments` in a field initializer or a static block, 0.155.0 lines 331-361) never runs for that reference. In `class C{x=([v=arguments]=[])}` the only error is the assign one, which the ep9 filter drops as a read; so the filter runs that class check itself and reports oxc's "'arguments' is not allowed in class field initializer" (or "static initialization block") instead (node v24.22.1: SyntaxError).

A second, related gap: the walk never meets a `ForInStatement` / `ForOfStatement`, so `for (eval of [])` and `for (arguments in {})` in strict code get no error. ep9 does not work around this one (no test262 test has it; noted as a known difference).

## Repro (pinned version)

oxc `=0.155.0` from crates.io (`oxc_allocator`, `oxc_parser`, `oxc_semantic`, `oxc_span`), Rust 1.99.1, release build, macOS arm64. Each line is parsed as a Script with a `"use strict";` line in front, then `SemanticBuilder::new().with_check_syntax_error(true).build(..)`; the program prints the first error or "accepted". node v24.22.1 runs the same strict text (indirect eval of `'"use strict";\n' + line`).

```rust
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;

fn main() {
    let path = std::env::args().nth(1).expect("usage: oxc-repro <file of lines>");
    for line in std::fs::read_to_string(path).unwrap().lines() {
        if line.is_empty() {
            continue;
        }
        let src = format!("\"use strict\";\n{line}");
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, &src, SourceType::script()).parse();
        let out = if parsed.diagnostics.has_errors() {
            format!("parse error: {}", parsed.diagnostics.errors().next().unwrap().message)
        } else {
            let built = SemanticBuilder::new()
                .with_check_syntax_error(true)
                .build(&parsed.program);
            match built.diagnostics.errors().next() {
                Some(e) => format!("error: {}", e.message),
                None => "accepted".into(),
            }
        };
        println!("{line:<45} -> {out}");
    }
}
```

| Line (strict) | oxc 0.155.0 | node v24.22.1 |
|---|---|---|
| `var v; [v = eval("1")] = [];` | Cannot assign to 'eval' in strict mode | ok, `v` is 1 |
| `var v; [v = eval] = [];` | Cannot assign to 'eval' in strict mode | ok |
| `var v; ({ v = eval("2") } = {});` | Cannot assign to 'eval' in strict mode | ok, `v` is 2 |
| `var v; ({ a: v = eval("3") } = {});` | Cannot assign to 'eval' in strict mode | ok, `v` is 3 |
| `var v; for ([v = eval("5")] of [[]]);` | Cannot assign to 'eval' in strict mode | ok, `v` is 5 |
| `var a = []; [a[eval("0")]] = [4];` | accepted | ok, `a[0]` is 4 |
| `var v; ({ [eval("'k'")]: v } = { k: 7 });` | Cannot assign to 'eval' in strict mode | ok, `v` is 7 |
| `var v; [v = () => arguments] = [];` | Cannot assign to 'arguments' in strict mode | ok |
| `[eval] = [];` | Cannot assign to 'eval' in strict mode | SyntaxError |
| `({eval} = {});` | Cannot assign to 'eval' in strict mode | SyntaxError |
| `eval++;` | Cannot assign to 'eval' in strict mode | SyntaxError |
| `for (eval of []);` | accepted | SyntaxError |
| `for (arguments in {});` | accepted | SyntaxError |

## Repro (upstream main, 2026-10-27, `def47b809338db8eb78e79e011af78111136d3ee`)

The same program with path dependencies on a shallow clone of main (commit date 2026-10-26T14:31:18Z): the output is identical to the pinned run (`diff` prints nothing). The checker code is unchanged on main.

## Text to post

> **Strict mode: reading `eval`/`arguments` inside a destructuring assignment target is reported as "Cannot assign to 'eval' in strict mode"**
>
> With `SemanticBuilder::with_check_syntax_error(true)` (oxc 0.155.0 and main), strict code like
>
> ```js
> "use strict";
> var v;
> [v = eval("1")] = [];            // default value
> ({ [eval("'k'")]: v } = { k: 1 }); // computed key
> [v = () => arguments] = [];      // nested function
> ```
>
> reports `Cannot assign to 'eval' in strict mode` (or `'arguments'`). The spec makes `eval`/`arguments` an early error only when the IdentifierReference itself is the assignment target (AssignmentTargetType, ECMA-262 13.1.1); here it is only read, and V8 (node 24) runs all three lines. `[eval] = []`, `({eval} = {})` and `eval++` are correctly rejected.
>
> Cause: `check_identifier_reference` in `crates/oxc_semantic/src/checker/javascript.rs` walks every ancestor and reports at the first `ArrayAssignmentTarget`, `ObjectAssignmentTarget`, `AssignmentTargetPropertyIdentifier` or `UpdateExpression`; it stops only at a member expression. A fix could report only when the reference is in a target position (a `SimpleAssignmentTarget::AssignmentTargetIdentifier`, or the binding of an `AssignmentTargetPropertyIdentifier`), for example by checking the parent node instead of walking all ancestors.
>
> Related: the same walk never sees a for-in/of head, so `"use strict"; for (eval of []);` and `for (arguments in {});` are accepted, while V8 reports `SyntaxError: Unexpected eval or arguments in strict mode`.
