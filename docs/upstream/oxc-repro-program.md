# Shared repro program for the oxc drafts

The program that the three oxc drafts quote. It is kept here so that anyone can run the repros again. It is not a workspace member and not built by CI.

- Verified by P0-OXC-VERIFY (M0 month 2, P0), 2026-10-21, Rust 1.99.1, release build, macOS arm64, node v24.22.0.
- Pinned run: oxc `=0.155.0` from crates.io (release commit `73e6b564737a29ecebf428d276421dcc492836df`, 2026-10-19).
- Main run: oxc main `f32360093c2ee18096fa14e0f48c48d75ca5a4a4` (2026-10-21T05:21:14Z; 21 commits ahead of the release commit, 0 behind), path dependencies on a shallow clone.
- The two runs print the same output. The only difference is the thread id in the stack overflow message.

## Layout

```
oxc-repro/
  src/main.rs        the program below
  run-all.sh         the driver below
  pin/Cargo.toml     oxc =0.155.0 from crates.io
  main/Cargo.toml    the same, with path dependencies on an oxc main checkout
```

`pin/Cargo.toml`:

```toml
[package]
name = "oxc-repro-pin"
version = "0.0.0"
edition = "2024"
publish = false

[[bin]]
name = "oxc-repro"
path = "../src/main.rs"

[dependencies]
oxc_allocator = "=0.155.0"
oxc_ast = "=0.155.0"
oxc_ast_visit = "=0.155.0"
oxc_codegen = "=0.155.0"
oxc_parser = "=0.155.0"
oxc_semantic = "=0.155.0"
oxc_span = "=0.155.0"

[profile.release]
debug = false

[workspace]
```

`main/Cargo.toml` is the same file with `name = "oxc-repro-main"` and each dependency as `{ path = "<oxc checkout>/crates/<crate>" }`.

Commands:

```sh
git clone --depth 1 https://github.com/oxc-project/oxc.git oxc-main
(cd pin  && CARGO_TARGET_DIR=../target-pin  cargo build --release)
(cd main && CARGO_TARGET_DIR=../target-main cargo build --release)
./run-all.sh target-pin/release/oxc-repro  > out-pin.txt  2>&1
./run-all.sh target-main/release/oxc-repro > out-main.txt 2>&1
diff <(sed -E 's/ \([0-9]+\) has/ has/' out-pin.txt) <(sed -E 's/ \([0-9]+\) has/ has/' out-main.txt)   # no output
```

## src/main.rs

```rust
//! Repro program for the oxc upstream drafts.
//!
//! Modes:
//!   check <src>                 parse as script (+ semantic syntax check); print the errors of the
//!                               first stage that has any, or "accepted"
//!   ast <src>                   parse, then semantic; print error counts, fatal_error, statement count,
//!                               counts of the rows 1-4 nodes, and the codegen of the AST
//!   depth <form> <n> <full|parse>   run one depth case in a child process on an 8 MiB thread
//!   bisect <form> <lo> <hi> <full|parse>  find the first failing depth in [lo, hi]
//!   depth-child <form> <n> <full|parse>   (internal)

use std::process::Command;

use oxc_allocator::Allocator;
use oxc_ast::ast::{CallExpression, Expression, NewTarget, PrivateFieldExpression, StaticMemberExpression};
use oxc_ast_visit::{Visit, walk};
use oxc_codegen::Codegen;
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;

fn opts() -> ParseOptions {
    ParseOptions { parse_regular_expression: true, ..ParseOptions::default() }
}

fn check(src: &str) -> String {
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, SourceType::script()).with_options(opts()).parse();
    if ret.diagnostics.has_errors() {
        let msgs: Vec<String> = ret.diagnostics.errors().map(|d| d.message.to_string()).collect();
        return format!("parser: {}", msgs.join(" | "));
    }
    let sem = SemanticBuilder::new().with_check_syntax_error(true).build(&ret.program);
    if sem.diagnostics.has_errors() {
        let msgs: Vec<String> = sem.diagnostics.errors().map(|d| d.message.to_string()).collect();
        return format!("semantic: {}", msgs.join(" | "));
    }
    "accepted".to_string()
}

#[derive(Default)]
struct Counts {
    new_target: u32,
    super_member: u32,
    super_call: u32,
    private_field: u32,
}

impl<'a> Visit<'a> for Counts {
    fn visit_new_target(&mut self, it: &NewTarget) {
        self.new_target += 1;
        walk::walk_new_target(self, it);
    }
    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if matches!(it.object, Expression::Super(_)) {
            self.super_member += 1;
        }
        walk::walk_static_member_expression(self, it);
    }
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if matches!(it.callee, Expression::Super(_)) {
            self.super_call += 1;
        }
        walk::walk_call_expression(self, it);
    }
    fn visit_private_field_expression(&mut self, it: &PrivateFieldExpression<'a>) {
        self.private_field += 1;
        walk::walk_private_field_expression(self, it);
    }
}

fn ast(src: &str) -> String {
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, SourceType::script()).with_options(opts()).parse();
    let mut c = Counts::default();
    c.visit_program(&ret.program);
    let code = Codegen::new().build(&ret.program).code;
    let sem = SemanticBuilder::new().with_check_syntax_error(true).build(&ret.program);
    format!(
        "semantic_errors={} parser_errors={} fatal_error={} statements={} new.target={} super.x={} super()={} #field={} codegen={:?}",
        sem.diagnostics.errors().count(),
        ret.diagnostics.errors().count(),
        ret.fatal_error,
        ret.program.body.len(),
        c.new_target,
        c.super_member,
        c.super_call,
        c.private_field,
        code.trim_end()
    )
}

fn depth_src(form: &str, n: usize) -> String {
    match form {
        "array" => format!("{}{}", "[".repeat(n), "]".repeat(n)),
        "paren" => format!("{}0{}", "(".repeat(n), ")".repeat(n)),
        "cond" => format!("{}0", "a ? b : ".repeat(n)),
        "for" => format!("{};", "for (;;) ".repeat(n)),
        "blocks" => format!("{}{}", "{".repeat(n), "}".repeat(n)),
        _ => panic!("unknown form {form}"),
    }
}

fn depth_child(form: &str, n: usize, full: bool) {
    let src = depth_src(form, n);
    let h = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let alloc = Allocator::default();
            let ret = Parser::new(&alloc, &src, SourceType::script()).with_options(opts()).parse();
            let perr = ret.diagnostics.errors().count();
            let serr = if full {
                SemanticBuilder::new().with_check_syntax_error(true).build(&ret.program).diagnostics.errors().count()
            } else {
                0
            };
            (perr, serr)
        })
        .unwrap();
    let (perr, serr) = h.join().unwrap();
    println!("ok (parser errors {perr}, semantic errors {serr})");
}

/// Returns (ok, one-line description).
fn depth(form: &str, n: usize, mode: &str) -> (bool, String) {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["depth-child", form, &n.to_string(), mode])
        .output()
        .unwrap();
    if out.status.success() {
        (true, String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        use std::os::unix::process::ExitStatusExt;
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
        (false, format!("abort (signal {:?}): {first}", out.status.signal()))
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a[1].as_str() {
        "check" => println!("{}", check(&a[2])),
        "ast" => println!("{}", ast(&a[2])),
        "depth" => {
            let (_, s) = depth(&a[2], a[3].parse().unwrap(), &a[4]);
            println!("{} n={} {}: {s}", a[2], a[3], a[4]);
        }
        "bisect" => {
            let (mut lo, mut hi): (usize, usize) = (a[3].parse().unwrap(), a[4].parse().unwrap());
            // invariant: lo ok, hi fails
            assert!(depth(&a[2], lo, &a[5]).0, "lo must pass");
            assert!(!depth(&a[2], hi, &a[5]).0, "hi must fail");
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if depth(&a[2], mid, &a[5]).0 { lo = mid } else { hi = mid }
            }
            println!("{} {}: last ok {lo}, first abort {hi}", a[2], a[5]);
        }
        "depth-child" => depth_child(&a[2], a[3].parse().unwrap(), a[4] == "full"),
        m => panic!("unknown mode {m}"),
    }
}
```

## run-all.sh

```bash
#!/usr/bin/env bash
# usage: run-all.sh <path to oxc-repro binary>
set -u
B=$1
echo "## eval (check)"
for s in 'new.target' 'super.x' 'super()' 'this.#x' 'arguments' 'with (o) {}' '"use strict"; with (o) {}' 'return 1'; do
  printf '%-26s %s\n' "$s" "$("$B" check "$s")"
done
echo "## eval (ast, rows 1-4)"
for s in 'new.target' 'super.x' 'super()' 'this.#x'; do
  printf '%-12s %s\n' "$s" "$("$B" ast "$s")"
done
echo "## Function()"
"$B" check "$(printf '(function anonymous(/*\n) {\n*/){\n})')"
"$B" check "$(printf '(function anonymous(\n) {\n}); (function(){\n})')"
echo "## depth (8 MiB thread, child process per case)"
for f in array paren cond for blocks; do for n in 1000 10000 50000; do for m in full parse; do "$B" depth $f $n $m; done; done; done
echo "## bisect (parse only)"
"$B" bisect array 1000 10000 parse
"$B" bisect paren 1000 10000 parse
```

## Output (oxc =0.155.0; oxc main is the same)

Thread ids are removed from the stack overflow lines.

```
## eval (check)
new.target                 parser: Unexpected new.target expression
super.x                    semantic: 'super' can only be referenced in members of derived classes or object literal expressions.
super()                    semantic: Super calls are not permitted outside constructors or in nested functions inside constructors.
this.#x                    semantic: Private identifier '#x' is not allowed outside class bodies
arguments                  accepted
with (o) {}                accepted
"use strict"; with (o) {}  semantic: 'with' statements are not allowed
return 1                   parser: A 'return' statement can only be used within a function body.
## eval (ast, rows 1-4)
new.target   semantic_errors=0 parser_errors=1 fatal_error=false statements=1 new.target=1 super.x=0 super()=0 #field=0 codegen="new.target;"
super.x      semantic_errors=1 parser_errors=0 fatal_error=false statements=1 new.target=0 super.x=1 super()=0 #field=0 codegen="super.x;"
super()      semantic_errors=1 parser_errors=0 fatal_error=false statements=1 new.target=0 super.x=0 super()=1 #field=0 codegen="super();"
this.#x      semantic_errors=1 parser_errors=0 fatal_error=false statements=1 new.target=0 super.x=0 super()=0 #field=1 codegen="this.#x;"
## Function()
accepted
accepted
## depth (8 MiB thread, child process per case)
array n=1000 full: ok (parser errors 0, semantic errors 0)
array n=1000 parse: ok (parser errors 0, semantic errors 0)
array n=10000 full: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
array n=10000 parse: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
array n=50000 full: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
array n=50000 parse: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
paren n=1000 full: ok (parser errors 0, semantic errors 0)
paren n=1000 parse: ok (parser errors 0, semantic errors 0)
paren n=10000 full: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
paren n=10000 parse: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
paren n=50000 full: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
paren n=50000 parse: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
cond n=1000 full: ok (parser errors 0, semantic errors 0)
cond n=1000 parse: ok (parser errors 0, semantic errors 0)
cond n=10000 full: ok (parser errors 0, semantic errors 0)
cond n=10000 parse: ok (parser errors 0, semantic errors 0)
cond n=50000 full: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
cond n=50000 parse: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
for n=1000 full: ok (parser errors 0, semantic errors 0)
for n=1000 parse: ok (parser errors 0, semantic errors 0)
for n=10000 full: ok (parser errors 0, semantic errors 0)
for n=10000 parse: ok (parser errors 0, semantic errors 0)
for n=50000 full: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
for n=50000 parse: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
blocks n=1000 full: ok (parser errors 0, semantic errors 0)
blocks n=1000 parse: ok (parser errors 0, semantic errors 0)
blocks n=10000 full: ok (parser errors 0, semantic errors 0)
blocks n=10000 parse: ok (parser errors 0, semantic errors 0)
blocks n=50000 full: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
blocks n=50000 parse: abort (signal Some(6)): thread '<unknown>' has overflowed its stack
## bisect (parse only)
array parse: last ok 5465, first abort 5466
paren parse: last ok 5464, first abort 5465
```

The same eval and `Function()` cases in node v24.22.0 (direct eval inside the stated context; `new Function(p, body)`):

```
1 new.target in function -> ok undefined
2 super.x in method -> ok number
3 super() in derived ctor -> ok object
4 this.#x in class with #x -> ok number
5 arguments in field init -> SyntaxError 'arguments' is not allowed in class field initializer or static initialization block
6 with in strict caller -> SyntaxError Strict mode code may not include a with statement
["/*","*/){"] SyntaxError Unexpected end of arg string
["","}); (function(){"] SyntaxError Single function literal required
```

The node script:

```js
const r = (name, f) => { try { const v = f(); console.log(name, '-> ok', typeof v); } catch (e) { console.log(name, '->', e.constructor.name, e.message); } };
r('1 new.target in function', () => (function () { return eval('new.target'); })());
r('2 super.x in method', () => ({ __proto__: { x: 1 }, m() { return eval('super.x'); } }).m());
r('3 super() in derived ctor', () => { class B {} class C extends B { constructor() { eval('super()'); } } return new C(); });
r('4 this.#x in class with #x', () => { class C { #x = 1; m() { return eval('this.#x'); } } return new C().m(); });
r('5 arguments in field init', () => { class C { f = eval('arguments'); } return new C(); });
r('6 with in strict caller', () => (function () { 'use strict'; var o = {}; return eval('with (o) {}'); })());
```
