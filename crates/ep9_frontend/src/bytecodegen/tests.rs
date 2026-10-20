//! Golden disassembly tests and unit tests.
//!
//! Each golden case compiles a small program and compares the full disassembly with
//! `golden/<name>.txt`. Re-bless after a reviewed change:
//!
//! ```text
//! EP9_BLESS=1 cargo test -p ep9_frontend --lib bytecodegen::tests
//! ```

use std::fmt::Write as _;
use std::path::PathBuf;

use ep9_bytecode::{Opcode, OpcodeFlags, VerifiedUnit};

use super::{CodegenError, Determinism, check_determinism};
use crate::{SourceOpts, Strictness, compile, compile_module, compile_script};

#[derive(Clone, Copy)]
enum Mode {
    Sloppy,
    Strict,
    Module,
}

impl Mode {
    fn opts(self) -> SourceOpts {
        match self {
            Mode::Sloppy => SourceOpts::script(Strictness::Sloppy),
            Mode::Strict => SourceOpts::script(Strictness::Strict),
            Mode::Module => SourceOpts::module(),
        }
    }
}

use Mode::{Module, Sloppy, Strict};

/// (name, mode, source). The name is the golden file name.
const CASES: &[(&str, Mode, &str)] = &[
    // ---- literals ----
    (
        "lit-numbers",
        Sloppy,
        "0; 1; -1; 2147483647; 2147483648; 1.5; -0; 1e300; 0.1 + 0.2;",
    ),
    (
        "lit-strings",
        Sloppy,
        r#"0; "a"; "\u00e9"; "\u4e2d"; "\uD800"; "\uD83D\uDE00"; ""; 'x' + "y"; "\0\n";"#,
    ),
    ("lit-bigint", Sloppy, "0n; 42n; 18446744073709551616n; -5n;"),
    ("lit-regexp", Sloppy, "/a+b/; /x/dgimsuy; /[a]/v;"),
    (
        "lit-template",
        Sloppy,
        "var a, b; `x`; `${a}`; `a${a}b${b}c`; `${a}${b}`;",
    ),
    (
        "lit-tagged-template",
        Sloppy,
        "var t, o; t`a${1}b`; o.m`\\u{`; t``;",
    ),
    ("lit-oddballs", Sloppy, "null; true; false; void 0; this;"),
    (
        "lit-array",
        Sloppy,
        "var x, y; []; [1, 2, 3]; [x, 1, y]; [1, , 2]; [[1], [2, 3]]; [...x]; [1, ...x, y, , ];",
    ),
    (
        "lit-object",
        Sloppy,
        "var x, k; ({}); ({a: 1, b: 'c'}); ({a: x, b: 2}); ({a: 1, a: 2}); ({[k]: 1, b: x}); ({x}); ({...x}); ({a: 1, ...x, b: 2}); ({__proto__: null, a: 1}); ({'__proto__': x}); ({__proto__}) ; ({1: 2, 1.5: 3});",
    ),
    (
        "lit-object-methods",
        Sloppy,
        "var k; ({m() { return 1; }, get g() { return 2; }, set g(v) {}, [k]() {}, get [k]() {}, f: function() {}, a: () => 1, [k]: function() {}});",
    ),
    (
        "lit-object-super",
        Sloppy,
        "({m() { return super.x; }, n() { super.y = 1; return super[1]; }});",
    ),
    // ---- operators ----
    (
        "op-arith",
        Sloppy,
        "function f(a, b) { return [a + b, a - 1, a * b, a / 2, a % b, a ** 2, a | b, a & 1, a ^ b, a << 2, a >> b, a >>> 1]; }",
    ),
    (
        "op-compare",
        Sloppy,
        "function f(a, b) { return [a == b, a != b, a === b, a !== b, a < b, a <= b, a > b, a >= b, a in b, a instanceof b]; }",
    ),
    (
        "op-unary",
        Sloppy,
        "function f(a) { return [-a, +a, ~a, !a, typeof a, void a, -1, -0]; }",
    ),
    (
        "op-typeof",
        Sloppy,
        "typeof undeclared; typeof undeclared === 'undefined'; let t; typeof t == 'number'; typeof t !== 'object';",
    ),
    (
        "op-delete",
        Sloppy,
        "var o, k; delete o.a; delete o[k]; delete x; delete o; delete 1; delete o?.a;",
    ),
    (
        "op-delete-strict",
        Strict,
        "var o, k; delete o.a; delete o[k];",
    ),
    (
        "op-logical",
        Sloppy,
        "function f(a, b, c) { return [a && b, a || b, a ?? b, a && b || c, a ? b : c]; }",
    ),
    ("op-comma", Sloppy, "var a, b; a = (1, 2, b);"),
    (
        "op-eval-order",
        Sloppy,
        "function f(a) { return a + (a = 1); }",
    ),
    (
        "op-null-compare",
        Sloppy,
        "function f(a) { return [a === null, a !== null, a == null]; }",
    ),
    // ---- assignment and update ----
    (
        "assign-simple",
        Sloppy,
        "var g; let l; function f(p) { var r; r = 1; p = 2; g = 3; l = 4; return r; }",
    ),
    (
        "assign-compound",
        Sloppy,
        "var g; function f(o, k, a) { a += 1; a -= o; g *= 2; o.x |= 1; o[k] **= a; }",
    ),
    (
        "assign-logical",
        Sloppy,
        "function f(o, a) { a ||= 1; a &&= 2; a ??= 3; o.x ??= 4; o[a] ||= 5; }",
    ),
    (
        "assign-update",
        Sloppy,
        "var g; function f(o, k, a) { a++; ++a; var x = a--; g++; o.x++; --o[k]; return x; }",
    ),
    (
        "assign-const",
        Sloppy,
        "const c = 1; function f() { c = 2; c += 3; }",
    ),
    (
        "assign-named-fn",
        Sloppy,
        "(function g() { g = 1; return g; }); (function h() { 'use strict'; h = 1; });",
    ),
    // ---- variables, scopes, closures ----
    (
        "var-tdz",
        Sloppy,
        "function f() { x; let x = 1; return () => x; }",
    ),
    (
        "var-tdz-loop",
        Sloppy,
        "function f() { for (let i = 0; i < 2; i++) { setTimeout(() => i); } }",
    ),
    (
        "var-closure",
        Sloppy,
        "function f(a) { var b = 1; let c = 2; return function g() { return a + b + c; }; }",
    ),
    (
        "var-global-lexical",
        Sloppy,
        "let a = 1; const b = 2; class C {} a = b; function g() { return a + b; }",
    ),
    (
        "var-annex-b",
        Sloppy,
        "function f() { { function g() {} } return g; } if (true) function h() {}",
    ),
    (
        "var-arguments",
        Sloppy,
        "function f(a) { a = 1; return arguments; } function g(a) { 'use strict'; return arguments[0]; }",
    ),
    (
        "var-catch-capture",
        Sloppy,
        "try {} catch (e) { (() => e)(); }",
    ),
    (
        "var-shadow-block",
        Sloppy,
        "function f() { let a = 1; { let a = 2; (() => a)(); } return a; }",
    ),
    ("var-script-this", Sloppy, "var f = () => this; this.x = 1;"),
    (
        "var-direct-eval",
        Sloppy,
        "function f(a) { var b; eval('a + b'); return b; }",
    ),
    ("var-with", Sloppy, "var o = {}; with (o) { x = 1; x; }"),
    // ---- functions ----
    (
        "fn-params",
        Sloppy,
        "function f(a, b = a, ...c) { return [a, b, c]; }",
    ),
    (
        "fn-params-pattern",
        Sloppy,
        "function f({a, b: [c] = []}, d) { return a + c + d; }",
    ),
    (
        "fn-params-split",
        Sloppy,
        "function f(a = () => b, b) { var b; var a; return a; }",
    ),
    (
        "fn-arrow",
        Sloppy,
        "var f = (a, b) => a + b; var g = () => { return this; }; var h = x => ({x});",
    ),
    (
        "fn-named-expr",
        Sloppy,
        "var f = function g() { return g; }; var h = function() {}; var i = () => {};",
    ),
    (
        "fn-new-target",
        Sloppy,
        "function F() { return new.target; } function G() { return () => new.target; }",
    ),
    // ---- control flow ----
    (
        "ctl-if",
        Sloppy,
        "function f(a, b) { if (a) return 1; else if (b && a) return 2; else { return 3; } }",
    ),
    (
        "ctl-while",
        Sloppy,
        "function f(a) { while (a) { a--; if (a == 3) continue; if (a == 1) break; } }",
    ),
    (
        "ctl-do-while",
        Sloppy,
        "function f(a) { do { a--; } while (a > 0); }",
    ),
    (
        "ctl-for",
        Sloppy,
        "function f() { var s = 0; for (var i = 0; i < 10; i++) s += i; for (;;) break; return s; }",
    ),
    (
        "ctl-for-let-closure",
        Sloppy,
        "function f() { var r = []; for (let i = 0; i < 3; i++) { r.push(() => i); if (i) continue; } return r; }",
    ),
    (
        "ctl-labels",
        Sloppy,
        "function f(a) { outer: for (var i = 0; i < 3; i++) { inner: for (;;) { if (a) continue outer; break outer; } } l: { if (a) break l; a = 1; } }",
    ),
    (
        "ctl-switch",
        Sloppy,
        "function f(a) { switch (a) { case 1: return 'one'; default: a = 0; case 2: { break; } case 3: let z = 3; (() => z)(); } return a; }",
    ),
    (
        "ctl-try-catch",
        Sloppy,
        "function f(g) { try { g(); } catch (e) { return e; } try { g(); } catch { } try { g(); } catch ({message}) { return message; } }",
    ),
    (
        "ctl-try-finally",
        Sloppy,
        "function f(g) { try { return g(); } finally { g(); } }",
    ),
    (
        "ctl-finally-break",
        Sloppy,
        "function f(g) { for (;;) { try { if (g()) break; continue; } finally { g(); } } }",
    ),
    (
        "ctl-finally-nested",
        Sloppy,
        "function f(g) { try { try { return 1; } finally { g(); } } finally { g(); } }",
    ),
    (
        "ctl-finally-context",
        Sloppy,
        "function f(g) { try { let x = 1; g(() => x); return x; } finally { g(); } }",
    ),
    (
        "ctl-throw",
        Sloppy,
        "function f(e) { throw e; } function g() { throw new Error('x'); }",
    ),
    ("ctl-debugger", Sloppy, "debugger;"),
    // ---- calls ----
    (
        "call-forms",
        Sloppy,
        "var f, o, k, a; f(); f(1); f(1, 2); f(1, 2, 3); o.m(); o.m(1); o.m(1, 2); o.m(1, 2, 3); o[k](a); (0, o.m)(); (o.m)();",
    ),
    (
        "call-spread",
        Sloppy,
        "var f, o, a; f(...a); f(1, ...a); f(...a, 1); o.m(...a); new f(...a); new f(...a, 2);",
    ),
    (
        "call-new",
        Sloppy,
        "var C, a; new C; new C(); new C(1, a); new new C()();",
    ),
    (
        "call-optional",
        Sloppy,
        "var o, f; o?.a; o?.[0]; o?.a.b; o?.m(); o.m?.(); f?.(); o?.a?.b?.(); (o?.a).b;",
    ),
    (
        "call-optional-paren",
        Sloppy,
        "var o; (o?.m)(); (o?.a[0])(1); (o?.m)`x`; (o?.m)?.();",
    ),
    (
        "call-eval",
        Sloppy,
        "function f(x) { return eval(x); } function g() { return eval(); } var e = eval; e('1');",
    ),
    // ---- iteration ----
    (
        "iter-for-in",
        Sloppy,
        "function f(o) { var r = []; for (var k in o) r.push(k); for (let j in o) (() => j)(); for (o.p in o) ; }",
    ),
    (
        "iter-for-in-annex-b",
        Sloppy,
        "function f(o) { for (var k = 1 in o) ; return k; }",
    ),
    (
        "iter-for-of",
        Sloppy,
        "function f(a) { var s = 0; for (const x of a) s += x; return s; }",
    ),
    (
        "iter-for-of-break",
        Sloppy,
        "function f(a) { for (let x of a) { if (x) break; (() => x)(); } }",
    ),
    (
        "iter-for-of-pattern",
        Sloppy,
        "function f(a) { for (const [k, v] of a) ; for ({x: a.y} of a) ; }",
    ),
    (
        "iter-for-of-return",
        Sloppy,
        "function f(a) { for (var x of a) return x; }",
    ),
    // ---- destructuring ----
    (
        "pat-object",
        Sloppy,
        "function f(o) { var {a, b: c, ['d']: e = 1, ...rest} = o; let {} = o; return [a, c, e, rest]; }",
    ),
    (
        "pat-array",
        Sloppy,
        "function f(a) { let [x, , y = 2, ...z] = a; const [] = a; return [x, y, z]; }",
    ),
    (
        "pat-nested",
        Sloppy,
        "function f(o) { const {a: [b, {c}], d: {e: [f] = []} = {}} = o; return b + c + f; }",
    ),
    (
        "pat-assign",
        Sloppy,
        "var a, b, o; [a, b] = [b, a]; ({a, b: o.x, ...o.y} = o); [o[0], ...o.z] = a;",
    ),
    (
        "pat-default-name",
        Sloppy,
        "var {f = function() {}} = {}; var [g = () => 1] = [];",
    ),
    // ---- classes ----
    (
        "class-base",
        Sloppy,
        "class A { constructor(x) { this.x = x; } m() { return this.x; } static s() { return 1; } get g() { return 1; } set g(v) {} static get sg() {} }",
    ),
    (
        "class-derived",
        Sloppy,
        "class A {} class B extends A { constructor() { super(1); this.y = 2; } m() { return super.m(); } }",
    ),
    (
        "class-default-ctor",
        Sloppy,
        "class A {} class B extends A {} var C = class {}; var D = class extends A {};",
    ),
    (
        "class-expression-name",
        Sloppy,
        "var C = class Inner { m() { return Inner; } }; var D = class { static n() { return D; } };",
    ),
    (
        "class-computed",
        Sloppy,
        "var k; class A { [k]() {} static [k + 1]() {} get [k]() {} 'str'() {} 1() {} }",
    ),
    (
        "class-super-arrow",
        Sloppy,
        "class A extends Object { constructor() { var f = () => super(); f(); } m() { return () => super.m(); } }",
    ),
    (
        "class-captured",
        Sloppy,
        "class A { m() { return () => A; } }",
    ),
    // ---- modules ----
    (
        "module-basic",
        Module,
        "import a, {b as c} from 'x'; export let d = a + c; export function e() { return d; } export default 1;",
    ),
    (
        "module-default-fn",
        Module,
        "export default function () { return 1; }",
    ),
    (
        "module-default-class",
        Module,
        "export default class { m() {} }",
    ),
    (
        "module-captured",
        Module,
        "let x = 1; export const y = () => x; x = 2;",
    ),
    // ---- strict and misc ----
    (
        "strict-script",
        Strict,
        "var a = 1; a = 2; function f() { return this; }",
    ),
    (
        "misc-positions",
        Sloppy,
        "var o;\no.a.b.c;\no.m(\n1);\nnew o.C();\n",
    ),
];

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/bytecodegen/golden")
}

fn compile_case(mode: Mode, src: &str) -> Result<VerifiedUnit, CodegenError> {
    compile(src, mode.opts(), super::CompileOptions::default())
}

fn render(name: &str, mode: Mode, src: &str) -> String {
    let mut out = String::new();
    let mode_name = match mode {
        Sloppy => "script (sloppy)",
        Strict => "script (strict)",
        Module => "module",
    };
    writeln!(out, "# {name}: {mode_name}").unwrap();
    for line in src.lines() {
        writeln!(out, "# | {line}").unwrap();
    }
    match compile_case(mode, src) {
        Ok(unit) => out.push_str(&ep9_bytecode::disassemble(unit.unit())),
        Err(e) => writeln!(out, "ERROR: {e}").unwrap(),
    }
    out
}

/// Every case matches its golden file (or writes it with `EP9_BLESS=1`).
#[test]
fn golden_disassembly() {
    let bless = std::env::var_os("EP9_BLESS").is_some();
    let dir = golden_dir();
    if bless {
        std::fs::create_dir_all(&dir).unwrap();
    }
    let mut failures = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for &(name, mode, src) in CASES {
        assert!(names.insert(name), "duplicate case {name}");
        let text = render(name, mode, src);
        assert!(!text.contains("ERROR:"), "{name} does not compile:\n{text}");
        let path = dir.join(format!("{name}.txt"));
        if bless {
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(expected) if expected == text => {}
            Ok(_) => failures.push(format!("{name}: differs (EP9_BLESS=1 to update)")),
            Err(_) => failures.push(format!("{name}: no golden file (EP9_BLESS=1 to create)")),
        }
    }
    if !bless {
        // No stale golden files.
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = file.strip_suffix(".txt")
                && !names.contains(stem)
            {
                failures.push(format!("{file}: stale golden file"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every golden case gives the same bytes twice (two threads).
#[test]
fn golden_cases_are_deterministic() {
    for &(name, mode, src) in CASES {
        match check_determinism(src, mode.opts()) {
            Ok(Determinism::Same(_)) => {}
            other => panic!("{name}: {other:?}"),
        }
    }
}

/// Instructions that may throw only on allocation failure: no own position needed.
fn allocation_only(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::CreateClosure
            | Opcode::CreateFunctionContext
            | Opcode::CreateBlockContext
            | Opcode::CreateCatchContext
            | Opcode::CreateEvalContext
            | Opcode::CreateWithContext
            | Opcode::CreateMappedArguments
            | Opcode::CreateUnmappedArguments
            | Opcode::CreateRestParameter
            | Opcode::CreateArrayLiteral
            | Opcode::CreateEmptyArrayLiteral
            | Opcode::CreateObjectLiteral
            | Opcode::CreateEmptyObjectLiteral
            | Opcode::CreateRegExpLiteral
            | Opcode::GetTemplateObject
    )
}

/// Throwing instructions of `unit` without their own position entry (G10).
pub(crate) fn unpositioned_throws(unit: &VerifiedUnit) -> Vec<String> {
    let unit = unit.unit();
    let mut out = Vec::new();
    for (fi, f) in unit.functions.iter().enumerate() {
        let positioned: std::collections::BTreeSet<u32> =
            f.positions.iter().map(|p| p.bytecode_offset).collect();
        for insn in ep9_bytecode::Instructions::new(&f.bytecode) {
            let insn = insn.expect("verified code decodes");
            let info = insn.opcode.info();
            if info.flags.contains(OpcodeFlags::THROWS)
                && !allocation_only(insn.opcode)
                && !positioned.contains(&insn.offset)
            {
                out.push(format!("f{fi} @{} {:?}", insn.offset, insn.opcode));
            }
        }
    }
    out
}

/// G10: every instruction that can throw (other than an allocation) has a position entry.
#[test]
fn throwing_instructions_have_positions() {
    let mut missing = Vec::new();
    for &(name, mode, src) in CASES {
        let unit = compile_case(mode, src).unwrap();
        for m in unpositioned_throws(&unit) {
            missing.push(format!("{name}: {m}"));
        }
    }
    assert!(
        missing.is_empty(),
        "{} missing:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

/// Every golden case has a consistent context chain (static check, see `ctx_check`).
#[test]
fn golden_cases_have_a_consistent_context_chain() {
    let mut errors = Vec::new();
    for &(name, mode, src) in CASES {
        let unit = compile_case(mode, src).unwrap();
        for e in super::ctx_check::context_errors(&unit) {
            errors.push(format!("{name}: {e}"));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// IteratorClose with a throw completion keeps the original exception even when the
/// `return` lookup throws (ECMA-262 7.4.11 step 4): the lookup is under the handler that
/// ignores errors, like the call. With other completions the lookup errors propagate.
#[test]
fn iterator_close_on_a_throw_ignores_errors_of_the_return_lookup() {
    use ep9_bytecode::{Constant, HandlerKind, Instructions, UnitString};
    let name = UnitString::new("return");
    for src in ["for (var x of a) {}", "var [y] = a;", "[z] = a;"] {
        let unit = compile_case(Sloppy, src).unwrap();
        let (u, f) = (unit.unit(), &unit.unit().functions[0]);
        let lookups: Vec<u32> = Instructions::new(&f.bytecode)
            .map(|i| i.expect("verified code decodes"))
            .filter(|i| {
                i.opcode == Opcode::GetNamedProperty
                    && matches!(&f.constants[i.unsigned(1) as usize],
                        Constant::String(s) if u.strings[s.index()] == name)
            })
            .map(|i| i.offset)
            .collect();
        let caught = |at: &&u32| {
            f.handlers
                .iter()
                .any(|h| h.kind == HandlerKind::Caught && (h.start..h.end).contains(*at))
        };
        let ignored = lookups.iter().filter(caught).count();
        assert_eq!((ignored, lookups.len() - ignored), (1, 1), "{src}");
    }
}

/// A `super[k]` reference keeps the key value; GetValue and PutValue convert it (ECMA-262
/// SuperProperty evaluation). So `super[k] = v()` calls `v` before ToPropertyKey(k) (in
/// StoreToSuper), and a compound assignment or update converts once, before the read, and the
/// store reuses the key (GetValue step 3.c). node converts twice there; JSC and test262 once.
#[test]
fn super_keys_are_converted_by_get_value_and_put_value() {
    use ep9_bytecode::{Instructions, RuntimeId};
    let src = "({ a() { super[k] = v(); }, b() { super[k] += v(); }, c() { super[k]++; }, \
               d() { return super[k](v()); } });";
    let unit = compile_case(Sloppy, src).unwrap();
    let steps = |f: usize| -> Vec<&str> {
        Instructions::new(&unit.unit().functions[f].bytecode)
            .map(|i| i.expect("verified code decodes"))
            .filter_map(|i| match i.opcode {
                Opcode::ToName => Some("ToName"),
                Opcode::CallRuntime => match RuntimeId::from_u16(i.unsigned(0) as u16) {
                    Some(r @ (RuntimeId::LoadKeyedFromSuper | RuntimeId::StoreToSuper)) => {
                        Some(r.info().name)
                    }
                    _ => None,
                },
                op if op.info().flags.contains(OpcodeFlags::CALL) => Some("call"),
                _ => None,
            })
            .collect()
    };
    assert_eq!(steps(1), ["call", "StoreToSuper"]);
    assert_eq!(
        steps(2),
        ["ToName", "LoadKeyedFromSuper", "call", "StoreToSuper"]
    );
    assert_eq!(steps(3), ["ToName", "LoadKeyedFromSuper", "StoreToSuper"]);
    assert_eq!(steps(4), ["LoadKeyedFromSuper", "call", "call"]);
}

#[test]
fn entry_points() {
    assert!(compile_script("1", Strictness::Sloppy).is_ok());
    assert!(compile_module("export {}").is_ok());
    // Early errors come back as the front end reports them.
    match compile_script("let let = 1;", Strictness::Sloppy) {
        Err(CodegenError::Compile(e)) => assert!(e.as_syntax().is_some()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn unsupported_constructs_name_the_feature() {
    for (src, feature) in [
        ("function* g() {}", "generator functions"),
        ("async function f() {}", "async functions"),
        ("(async () => 1)", "async functions"),
        ("class A { x = 1 }", "class fields"),
        ("class A { #x() {} }", "private names"),
        ("class A { static {} }", "class static blocks"),
        ("import('x')", "import()"),
    ] {
        match compile_script(src, Strictness::Sloppy) {
            Err(CodegenError::Unsupported(u)) => assert_eq!(u.feature, feature, "{src}"),
            other => panic!("{src}: {other:?}"),
        }
    }
    match compile_module("import.meta") {
        Err(CodegenError::Unsupported(u)) => assert_eq!(u.feature, "import.meta"),
        other => panic!("{other:?}"),
    }
    match compile_module("for await (const x of y) ;") {
        Err(CodegenError::Unsupported(u)) => assert_eq!(u.feature, "for await"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn unsupported_reports_a_position() {
    let Err(CodegenError::Unsupported(u)) =
        compile_script("1;\n  function* g() {}", Strictness::Sloppy)
    else {
        panic!("expected Unsupported");
    };
    assert_eq!((u.position.line, u.position.column), (2, 3));
    assert_eq!(u.span.start, 5);
}
