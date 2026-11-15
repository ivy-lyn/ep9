//! INTL W4 EVALSCRIPT (step A; route EVALSCRIPT-COMPLETION): `compile_script_with_completion`
//! compiles a Script whose function 0 returns the Script's completion value (ScriptEvaluation,
//! ECMA-262 §16.1.6), for the test262 host hook of `$262.evalScript`. The value comes from the
//! eval code rewrite (`eval.rs` E5): value-producing statements store into `.result`, which
//! is a register of function 0, never a global binding (a Script's top-level `let` is a global
//! lexical binding that the next Script sees). A Script compiled by `compile_script` does not
//! change, so the bytecodegen goldens and the corpus digest stay as they are.
//!
//! `cargo test -p ep9_frontend --test script_completion`

use ep9_bytecode::{VerifiedUnit, disassemble};
use ep9_frontend::{
    CodegenError, CompileError, Strictness, compile_script, compile_script_with_completion,
};

fn with_completion(src: &str, strictness: Strictness) -> VerifiedUnit {
    compile_script_with_completion(src, strictness)
        .unwrap_or_else(|e| panic!("{src:?}: compiles with the completion value: {e}"))
}

fn plain(src: &str, strictness: Strictness) -> VerifiedUnit {
    compile_script(src, strictness).unwrap_or_else(|e| panic!("{src:?}: compiles: {e}"))
}

/// The opcodes of function 0, in order (the disassembler's code lines of `f0`).
fn f0_opcodes(unit: &VerifiedUnit) -> Vec<String> {
    let text = disassemble(unit.unit());
    let mut out = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        if line.starts_with("function f0 ") {
            in_code = true;
            continue;
        }
        if !in_code {
            continue;
        }
        let t = line.trim_start();
        if line.starts_with("function ") || t.starts_with("constants") {
            break;
        }
        // `<offset> [S>|E>] <Opcode> ...`
        let mut words = t.split_whitespace();
        if words.next().is_some_and(|w| w.parse::<u32>().is_ok()) {
            let mut op = words.next().unwrap_or_default();
            if op == "S>" || op == "E>" {
                op = words.next().unwrap_or_default();
            }
            out.push(op.to_owned());
        }
    }
    out
}

/// The last two opcodes of function 0.
fn tail(unit: &VerifiedUnit) -> [String; 2] {
    let ops = f0_opcodes(unit);
    assert!(ops.len() >= 2, "function 0 has a body: {ops:?}");
    [ops[ops.len() - 2].clone(), ops[ops.len() - 1].clone()]
}

/// A Script with no value-producing statement gets no `.result` store: both entries give
/// the same bytes. This also holds for an empty strict Script: the synthetic "use strict"
/// directive (force_strict) is added after the rewrite, so it is not the completion value.
#[test]
fn no_value_statement_gives_the_same_unit() {
    let samples = [
        "",
        "var x;",
        "var x = 1, y;",
        "function f() { return 1; }",
        "let a = 1; const b = 2; class C {}",
        ";",
        "debugger;",
    ];
    for strictness in [Strictness::Sloppy, Strictness::Strict] {
        for src in samples {
            let a = plain(src, strictness);
            let b = with_completion(src, strictness);
            assert_eq!(a.to_bytes(), b.to_bytes(), "{src:?} {strictness:?}");
            assert_eq!(
                tail(&b),
                ["LdaUndefined", "Return"],
                "{src:?} {strictness:?}"
            );
        }
    }
}

/// A Script with a value returns it (a register load before `Return`); the same Script from
/// `compile_script` still returns undefined.
#[test]
fn value_statements_return_their_value() {
    let samples = [
        ("1; 2", Strictness::Sloppy),
        ("let a = 1; a + 1;", Strictness::Sloppy),
        ("(function () {})", Strictness::Strict),
        ("if (globalThis) 1; else 2;", Strictness::Sloppy),
        ("'use strict'; 3", Strictness::Sloppy),
        ("'a'", Strictness::Sloppy),
        ("do { 1; break; } while (0)", Strictness::Sloppy),
        ("try { 1 } finally { 2 }", Strictness::Strict),
        ("x: { 1; break x; }", Strictness::Sloppy),
    ];
    for (src, strictness) in samples {
        let unit = with_completion(src, strictness);
        assert_eq!(tail(&unit)[0], "Ldar", "{src:?}: {:?}", f0_opcodes(&unit));
        assert_eq!(tail(&unit)[1], "Return", "{src:?}");
        let old = plain(src, strictness);
        assert_eq!(tail(&old), ["LdaUndefined", "Return"], "{src:?}");
    }
}

/// `.result` (and a `try`/`finally` backup) never becomes a global binding: no name `.result`
/// in the unit (not in the strings, not in the scope 0 declarations of
/// GlobalDeclarationInstantiation, no global or lookup-slot store of it), with top-level `var`,
/// `let`, `const`, a sloppy direct eval and `with`.
#[test]
fn result_is_never_a_global_binding() {
    let samples = [
        ("1", Strictness::Sloppy),
        ("var v = 1; v;", Strictness::Sloppy),
        ("let l = 1; const c = 2; l + c;", Strictness::Strict),
        ("eval('var e = 1'); 2;", Strictness::Sloppy),
        ("var t = this; eval('t'); 3;", Strictness::Sloppy),
        ("with ({ w: 1 }) { w; }", Strictness::Sloppy),
        (
            "do { try { 1 } finally { 2; break } } while (0)",
            Strictness::Sloppy,
        ),
    ];
    for (src, strictness) in samples {
        let unit = with_completion(src, strictness);
        let text = disassemble(unit.unit());
        assert!(!text.contains(".result"), "{src:?}:\n{text}");
        let scope0 = text
            .lines()
            .find(|l| l.trim_start().starts_with("scope0 "))
            .expect("a scope 0 line");
        assert!(scope0.contains("script"), "{src:?}: {scope0}");
        let ops = f0_opcodes(&unit);
        assert_eq!(ops.last().map(String::as_str), Some("Return"), "{src:?}");
        assert_eq!(ops[ops.len() - 2], "Ldar", "{src:?}: {ops:?}");
    }
}

/// An early error is still an error, with the message `compile_script` gives.
#[test]
fn early_errors_stay_errors() {
    for (src, strictness) in [
        ("return 1;", Strictness::Sloppy),
        ("let a; let a;", Strictness::Sloppy),
        ("1; with (o) {}", Strictness::Strict),
        ("1; 08;", Strictness::Strict),
    ] {
        let message = |r: Result<VerifiedUnit, CodegenError>| match r {
            Err(CodegenError::Compile(CompileError::Syntax(e))) => e.first().message.clone(),
            Err(e) => panic!("{src:?}: not a SyntaxError: {e}"),
            Ok(_) => panic!("{src:?}: compiled"),
        };
        let want = message(compile_script(src, strictness));
        let got = message(compile_script_with_completion(src, strictness));
        assert_eq!(got, want, "{src:?}");
    }
}

/// Two builds (the second on another thread) give the same bytes (the H2 determinism rule).
#[test]
fn deterministic() {
    let src = "var a = 1; if (a) { a + 1; } else { 3 } \
               do { try { 1 } finally { 2; break } } while (0)";
    let first = with_completion(src, Strictness::Sloppy).to_bytes();
    let second = std::thread::spawn(move || with_completion(src, Strictness::Sloppy).to_bytes())
        .join()
        .expect("the second build");
    assert_eq!(first, second);
}
