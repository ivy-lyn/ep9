use super::*;

const SLOPPY: SourceOpts = SourceOpts::script(Strictness::Sloppy);
const STRICT: SourceOpts = SourceOpts::script(Strictness::Strict);
const MODULE: SourceOpts = SourceOpts::module();

fn ok(src: &str, opts: SourceOpts) -> bool {
    check_early_errors(src, opts).is_ok()
}

/// The early errors of `src`; panics on success or on a range error.
fn syntax_errors(src: &str, opts: SourceOpts) -> EarlyErrors {
    match check_early_errors(src, opts) {
        Err(CompileError::Syntax(errors)) => errors,
        other => panic!("expected early errors for {src:?}, got {other:?}"),
    }
}

/// Strict via `SourceOpts` must agree with strict via a `"use strict";` source prefix.
fn strict_both_ways(src: &str) -> bool {
    let by_opts = ok(src, STRICT);
    let by_prefix = ok(&format!("\"use strict\";\n{src}"), SLOPPY);
    assert_eq!(
        by_opts, by_prefix,
        "strict by opts and by prefix disagree on {src:?}"
    );
    by_opts
}

#[test]
fn opts_never_build_a_sloppy_module() {
    assert_eq!(MODULE.goal(), Goal::Module);
    assert_eq!(MODULE.strictness(), Strictness::Strict);
    assert_eq!(SLOPPY.goal(), Goal::Script);
    assert_eq!(SLOPPY.strictness(), Strictness::Sloppy);
    assert_eq!(STRICT.strictness(), Strictness::Strict);
}

#[test]
fn valid_code_parses_in_every_mode() {
    let src =
        "var a = [1, 2, 3].map((x) => x * 2);\nclass C { #p = 1; get p() { return this.#p; } }\n";
    assert!(ok(src, SLOPPY));
    assert!(ok(src, STRICT));
    assert!(ok(src, MODULE));
    assert!(ok("", SLOPPY));
    assert!(ok("", MODULE));
}

#[test]
fn goal_decides_module_syntax() {
    assert!(ok("export default 1;", MODULE));
    assert!(!ok("export default 1;", SLOPPY));
    assert!(!ok("export default 1;", STRICT));
    assert!(ok("import.meta;", MODULE));
    assert!(!ok("import.meta;", SLOPPY));
    // `await` is an identifier in scripts, even strict ones, and reserved in modules.
    assert!(ok("var await = 1;", SLOPPY));
    assert!(ok("var await = 1;", STRICT));
    assert!(!ok("var await = 1;", MODULE));
    // Top-level await is module only.
    assert!(ok("await 0;", MODULE));
    assert!(!ok("await 0;", SLOPPY));
}

#[test]
fn strictness_switch_matches_the_source_prefix() {
    // Each of these is valid sloppy code and an early error in strict code.
    let strict_errors = [
        "with (a) {}",
        "var x = 010;",
        "var s = '\\07';",
        "var s = '\\8';",
        "delete x;",
        "var yield;",
        "var let = 1;",
        "var static;",
        "function f(a, a) {}",
        "eval = 1;",
        "arguments++;",
        "if (a) function g() {}",
        "label: function h() {}",
        "function f() { var implements; }",
        "try {} catch (eval) {}",
    ];
    for src in strict_errors {
        assert!(ok(src, SLOPPY), "sloppy should accept {src:?}");
        assert!(!strict_both_ways(src), "strict should reject {src:?}");
        assert!(!ok(src, MODULE), "module should reject {src:?}");
    }
    // Valid in both modes.
    for src in [
        "var x = 0o10;",
        "'\\0';",
        "'use strict'; var x;",
        "function f() { 'use strict'; }",
    ] {
        assert!(ok(src, SLOPPY), "sloppy should accept {src:?}");
        assert!(strict_both_ways(src), "strict should accept {src:?}");
    }
}

#[test]
fn source_directive_still_makes_sloppy_code_strict() {
    assert!(!ok("'use strict'; with (a) {}", SLOPPY));
    assert!(!ok("function f() { 'use strict'; with (a) {} }", SLOPPY));
    // A directive after an octal escape string: the escape is an error too (ES2026 14.1.1).
    assert!(!ok("'\\07'; 'use strict';", SLOPPY));
}

#[test]
fn strict_switch_keeps_hashbang_and_html_comments_valid() {
    // A source prefix would break both of these; the AST directive does not.
    let hashbang = "#!/usr/bin/env ep9\nvar x = 1;";
    assert!(ok(hashbang, SLOPPY));
    assert!(ok(hashbang, STRICT));
    assert!(!ok("#!/usr/bin/env ep9\nwith (a) {}", STRICT));
    // Annex B HTML-like comments are valid in strict scripts and invalid in modules.
    let html = "-->comment at the start of the input\nvar x;";
    assert!(ok(html, SLOPPY));
    assert!(ok(html, STRICT));
    assert!(!ok(html, MODULE));
    assert!(ok("x = 1 <!-- y\n", STRICT));
}

#[test]
fn semantic_checks_run() {
    // oxc_parser accepts these; oxc_semantic rejects them.
    for src in [
        "let a; let a;",
        "break nowhere;",
        "a: a: ;",
        "class C { constructor() {} constructor() {} }",
        "class C { m() { this.#missing; } }",
        "function f() { super.x; }",
        "new.target;",
    ] {
        assert!(!ok(src, SLOPPY), "sloppy should reject {src:?}");
    }
}

#[test]
fn regex_literals_are_validated() {
    for src in [
        "/(?<a>.)\\k<b>/u;",
        "/\\p{NotAProperty}/u;",
        "/a{2,1}/;",
        "/[b-a]/;",
        "/(?<n>a)(?<n>b)/;",
        "/a/gg;",
    ] {
        assert!(!ok(src, SLOPPY), "should reject {src:?}");
    }
    for src in [
        "/(?<a>.)\\k<a>/u;",
        "/\\p{Script=Latin}/u;",
        "/a{1,2}/;",
        "/[\\d-z]/;",
        "/\\8/;",
    ] {
        assert!(ok(src, SLOPPY), "should accept {src:?}");
    }
}

#[test]
fn annex_b_sloppy_forms() {
    assert!(ok("if (a) function f() {}", SLOPPY));
    assert!(ok("var s = '\\101';", SLOPPY));
    assert!(ok("/{/;", SLOPPY));
    assert!(!ok("/{/u;", SLOPPY));
    assert!(ok("label: function f() {}", SLOPPY));
}

#[test]
fn errors_carry_span_line_and_utf16_column() {
    // Line 3. Before `with`: U+00E9 (2 bytes, 1 unit), U+1F600 (4 bytes, 2 units), then 3 spaces.
    let src = "'use strict';\n\n'\u{e9}\u{1F600}';   with (a) {}";
    let err = syntax_errors(src, SLOPPY);
    let first = err.first();
    let span = first.span.expect("a span");
    let with_at = u32::try_from(src.find("with").unwrap()).unwrap();
    assert_eq!(span.start, with_at, "{first:?}");
    assert!(span.end > span.start);
    // Column: ' (1) + e-acute (1) + emoji (2) + ' (1) + ; (1) + 3 spaces = 9 units before `with`.
    assert_eq!(
        first.position,
        Some(Position {
            line: 3,
            column: 10
        }),
        "{first:?}"
    );
    assert!(!first.message.is_empty());
    assert_eq!(
        err.to_string(),
        format!("3:10: SyntaxError: {}", first.message)
    );
}

#[test]
fn parse_errors_are_reported_without_semantic() {
    let err = syntax_errors("var = 1;\nlet a; let a;", SLOPPY);
    // The parser error stops the check, so the semantic redeclaration is not reported.
    assert!(
        err.as_slice()
            .iter()
            .all(|e| e.position.is_some_and(|p| p.line == 1)),
        "{err:?}"
    );
    assert_eq!(err.len(), err.as_slice().len());
    assert!(!err.is_empty());
}

#[test]
fn every_error_is_returned() {
    let err = syntax_errors("let a; let a; let b; let b;", SLOPPY);
    assert_eq!(err.len(), 2, "{err:?}");
    assert!(err.to_string().ends_with("(+1 more)"));
    assert_eq!(err.clone().into_vec().len(), 2);
}

#[test]
fn deep_nesting_needs_no_big_caller_stack() {
    // The front end hops to its sized-stack worker, so a 2,000-level input is fine on the default
    // test thread stack (2 MiB), which would overflow in a debug parse on the caller's thread.
    let depth = 2_000;
    let src = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    assert!(ok(&src, SLOPPY));
}

#[test]
fn compile_error_display_keeps_the_syntax_error_text() {
    let err = check_early_errors("let a; let a;", SLOPPY).unwrap_err();
    let syntax = err.as_syntax().expect("a syntax error");
    assert_eq!(err.to_string(), syntax.to_string());
    assert!(err.as_range().is_none());
}

/// H01 rule: no oxc type in the public API. A `pub` item (not `pub(crate)`) never names an oxc
/// crate, so the AST and oxc ids cannot leave `ep9_frontend`.
#[test]
fn no_public_item_names_an_oxc_type() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root];
    let mut checked = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let line = line.trim_start();
                if line.starts_with("pub ") && !line.starts_with("pub(") {
                    checked += 1;
                    assert!(
                        !line.contains("oxc"),
                        "{}:{}: public item names oxc: {line}",
                        path.display(),
                        n + 1
                    );
                }
            }
        }
    }
    assert!(checked > 100, "only {checked} public lines found");
}

// ---- the lone-surrogate shim (M7-P2 FE-SHIM; H05-Q3; surrogate.rs) -------------------------
// These tests write the shim text by hand (`shim_text`, the form the VM gives the hook) and
// read the unit, so they also run on a tree without the shim (where they fail).

/// The shim text of `units`: U+FDD0 + 4 lower-case hex digits for a lone surrogate, U+FDD0
/// twice for U+FDD0.
fn shim_text(units: &[u16]) -> String {
    let mut out = String::new();
    for c in char::decode_utf16(units.iter().copied()) {
        match c {
            Ok('\u{FDD0}') => out.push_str("\u{FDD0}\u{FDD0}"),
            Ok(c) => out.push(c),
            Err(e) => out.push_str(&format!("\u{FDD0}{:04x}", e.unpaired_surrogate())),
        }
    }
    out
}

/// `parts` joined as WTF-16: text, or one code unit.
enum P<'a> {
    T(&'a str),
    U(u16),
}

fn wtf16(parts: &[P<'_>]) -> Vec<u16> {
    let mut out = Vec::new();
    for p in parts {
        match p {
            P::T(t) => out.extend(t.encode_utf16()),
            P::U(u) => out.push(*u),
        }
    }
    out
}

fn eval_shim(parts: &[P<'_>]) -> Result<ep9_bytecode::VerifiedUnit, CodegenError> {
    compile_eval(
        &shim_text(&wtf16(parts)),
        &ep9_bytecode::EvalContext::default(),
        CompileOptions::default(),
    )
}

/// The unit has a string with exactly the code units of `parts`.
fn has_string(unit: &ep9_bytecode::VerifiedUnit, parts: &[P<'_>]) -> bool {
    let want = ep9_bytecode::UnitString::from_utf16(&wtf16(parts));
    unit.strings.contains(&want)
}

fn is_syntax(r: &Result<ep9_bytecode::VerifiedUnit, CodegenError>) -> bool {
    matches!(r, Err(CodegenError::Compile(CompileError::Syntax(_))))
}

/// S7.4_A5 and S7.4_A6: every lone surrogate is fine in a `//` and a `/* */` comment.
#[test]
fn shim_every_lone_surrogate_in_a_comment_compiles() {
    for u in 0xD800..=0xDFFF_u16 {
        let line = eval_shim(&[P::T("//var "), P::U(u), P::T("yy = -1")]);
        assert!(line.is_ok(), "{u:04x} in a // comment: {line:?}");
        let block = eval_shim(&[P::T("/*var "), P::U(u), P::T("xx = 1*/")]);
        assert!(block.is_ok(), "{u:04x} in a /* */ comment: {block:?}");
    }
}

/// Outside comments and literals a lone surrogate is a SyntaxError, and the column of a later
/// error counts it as 1 UTF-16 unit.
#[test]
fn shim_lone_surrogate_outside_literals_is_a_syntax_error() {
    assert!(is_syntax(&eval_shim(&[
        P::T("var a"),
        P::U(0xD800),
        P::T(";")
    ])));
    assert!(is_syntax(&eval_shim(&[P::U(0xDC00)])));
    let r = eval_shim(&[P::T("'"), P::U(0xD800), P::T("'; )")]);
    let Err(CodegenError::Compile(CompileError::Syntax(e))) = r else {
        panic!("expected a SyntaxError: {r:?}");
    };
    // `'x'; )`: the `)` is UTF-16 column 6 (1-based) of line 1.
    assert_eq!(e.first().position.map(|p| (p.line, p.column)), Some((1, 6)));
}

/// String literals and the completion value of a directive get the real code units: a lone
/// surrogate, a lone surrogate after `\`, and an escaped lead before a raw trail (a pair in
/// the value).
#[test]
fn shim_string_literals_get_the_real_code_units() {
    let u = eval_shim(&[P::T("var s = 'a"), P::U(0xD800), P::T("b';")]).expect("compiles");
    assert!(has_string(&u, &[P::T("a"), P::U(0xD800), P::T("b")]));
    let u = eval_shim(&[P::T("'\\"), P::U(0xDFFF), P::T("'")]).expect("compiles");
    assert!(has_string(&u, &[P::U(0xDFFF)]));
    let u = eval_shim(&[P::T("x = '\\uD83D"), P::U(0xDC38), P::T("'")]).expect("compiles");
    assert!(has_string(&u, &[P::U(0xD83D), P::U(0xDC38)]));
    let u = eval_shim(&[P::T("({'k"), P::U(0xDBFF), P::T("': 1})")]).expect("compiles");
    assert!(has_string(&u, &[P::T("k"), P::U(0xDBFF)]));
}

/// Template literals: the cooked value and the raw value (`String.raw`, CR LF -> LF) get the
/// real code units.
#[test]
fn shim_template_cooked_and_raw_values() {
    let u = eval_shim(&[P::T("`x"), P::U(0xDC00), P::T("`")]).expect("compiles");
    assert!(has_string(&u, &[P::T("x"), P::U(0xDC00)]));
    let u =
        eval_shim(&[P::T("String.raw`a\\"), P::U(0xD801), P::T("\r\n${1}`")]).expect("compiles");
    assert!(has_string(&u, &[P::T("a\\"), P::U(0xD801), P::T("\n")]));
    assert!(has_string(&u, &[P::T("a"), P::U(0xD801), P::T("\n")]));
}

/// Regex literals: the pattern keeps the real code units, and the check sees their values
/// (a range of two lone surrogates, a lone surrogate in a group name, after `\` in Unicode
/// mode).
#[test]
fn shim_regex_literals() {
    let u = eval_shim(&[P::T("/"), P::U(0xD800), P::T("/")]).expect("compiles");
    assert!(has_string(&u, &[P::U(0xD800)]));
    let u = eval_shim(&[P::T("/\\uD83D"), P::U(0xDC38), P::T("/u")]).expect("compiles");
    assert!(has_string(&u, &[P::T("\\uD83D"), P::U(0xDC38)]));
    // In order (DBFF <= DC00), but U+FFFD > U+DC00: the U+FFFD check alone would fail it.
    assert!(eval_shim(&[P::T("/["), P::U(0xDBFF), P::T("-\\uDC00]/")]).is_ok());
    // Out of order (DC00 > DBFF), but U+FFFD-U+FFFD is in order.
    assert!(is_syntax(&eval_shim(&[
        P::T("/["),
        P::U(0xDC00),
        P::T("-"),
        P::U(0xDBFF),
        P::T("]/")
    ])));
    // A lone surrogate is never a group name character, also before an escaped trail.
    for flags in ["", "u"] {
        let name = eval_shim(&[P::T("/(?<a"), P::U(0xD801), P::T(">.)/"), P::T(flags)]);
        assert!(is_syntax(&name), "/{flags}: {name:?}");
        let pair = eval_shim(&[
            P::T("/(?<a"),
            P::U(0xD801),
            P::T("\\uDCA4>.)/"),
            P::T(flags),
        ]);
        assert!(is_syntax(&pair), "/{flags}: {pair:?}");
    }
    // The parser's own error at a regex token stays.
    assert!(is_syntax(&eval_shim(&[
        P::T("if /"),
        P::U(0xD800),
        P::T("/ x")
    ])));
    assert!(is_syntax(&eval_shim(&[
        P::T("/"),
        P::U(0xD800),
        P::T("/gg")
    ])));
    assert!(eval_shim(&[P::T("/\\"), P::U(0xD800), P::T("/")]).is_ok());
    assert!(is_syntax(&eval_shim(&[
        P::T("/\\"),
        P::U(0xD800),
        P::T("/u")
    ])));
}

/// `\k<` is a group name only in a pattern with named groups (Codex r1 FE-SHIM 3): with no
/// u or v flag and no `(?<name>` group, `\k` is an identity escape (B.1.2), so `[z-` and a
/// lone surrogate after it are a class in order (node: `/\k<[z-\uD800]>/` compiles). With a
/// group or the u flag, `\k<[z-...]>` is a bad group name: a SyntaxError.
#[test]
fn shim_k_escape_is_a_group_name_only_with_named_groups() {
    let plain = eval_shim(&[P::T("/\\k<[z-"), P::U(0xD800), P::T("]>/")]);
    assert!(plain.is_ok(), "{plain:?}");
    let u = plain.expect("compiles");
    assert!(has_string(&u, &[P::T("\\k<[z-"), P::U(0xD800), P::T("]>")]));
    for (head, flags) in [("/(?<a>.)\\k<[z-", ""), ("/\\k<[z-", "u")] {
        let r = eval_shim(&[P::T(head), P::U(0xD800), P::T("]>/"), P::T(flags)]);
        assert!(is_syntax(&r), "{head} {flags}: {r:?}");
    }
}

/// A real U+FDD0 (shim text U+FDD0 U+FDD0) stays U+FDD0, also before 4 hex digits; a string
/// with U+FDD1 runs keeps its value next to a placeholder (surrogate.rs S5).
#[test]
fn shim_real_fdd0_and_fdd1_strings_keep_their_value() {
    let u = eval_shim(&[P::T("'\u{FDD0}d800'")]).expect("compiles");
    assert!(has_string(&u, &[P::T("\u{FDD0}d800")]));
    let u = eval_shim(&[
        P::T("['\u{FDD1}0', '\\uFDD1\\uFDD10', /"),
        P::U(0xD800),
        P::T("/]"),
    ])
    .expect("compiles");
    assert!(has_string(&u, &[P::T("\u{FDD1}0")]));
    assert!(has_string(&u, &[P::T("\u{FDD1}\u{FDD1}0")]));
    assert!(has_string(&u, &[P::U(0xD800)]));
    assert_eq!(
        placeholders_left(&u),
        0,
        "a placeholder is left in the unit"
    );
}

/// The unit strings of the placeholder form U+FDD1, decimal digits, U+FDD1 (surrogate.rs S5).
fn placeholders_left(unit: &ep9_bytecode::VerifiedUnit) -> usize {
    unit.strings
        .iter()
        .filter(|s| {
            let u = s.to_utf16();
            u.len() > 2
                && u[0] == 0xFDD1
                && u[u.len() - 1] == 0xFDD1
                && u[1..u.len() - 1]
                    .iter()
                    .all(|&c| (u16::from(b'0')..=u16::from(b'9')).contains(&c))
        })
        .count()
}

/// Values of the placeholder form keep their value next to placeholders, also when they come
/// after the regex literal that needs a placeholder: the numbers skip them (surrogate.rs S5).
#[test]
fn shim_values_of_the_placeholder_form_keep_their_value() {
    let u = eval_shim(&[
        P::T("[/"),
        P::U(0xD800),
        P::T("/, /"),
        P::U(0xDC00),
        P::T("/, '\\uFDD10\\uFDD1', '\u{FDD1}1\u{FDD1}', `\\u{FDD1}2\\u{fdd1}`]"),
    ])
    .expect("compiles");
    assert!(has_string(&u, &[P::U(0xD800)]));
    assert!(has_string(&u, &[P::U(0xDC00)]));
    for n in ["0", "1", "2"] {
        assert!(
            has_string(&u, &[P::T("\u{FDD1}"), P::T(n), P::T("\u{FDD1}")]),
            "{n}"
        );
    }
    assert_eq!(placeholders_left(&u), 3, "only the 3 values");
}

/// The placeholder work does not grow with the U+FDD1 of the text (Codex r1 FE-SHIM 2): K =
/// 2,000 U+FDD1 in a comment and N = 2,000 regex literals with a lone surrogate. Before, each
/// placeholder held K + 1 U+FDD1 and `finish` built a unit string per (unit string,
/// placeholder) pair: about 95 s in a debug build. The wait of 20 s only stops a test that
/// hangs.
#[test]
fn shim_placeholders_do_not_grow_with_the_text() {
    const K: usize = 2000;
    const N: usize = 2000;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut units: Vec<u16> = Vec::new();
        units.extend("/*".encode_utf16());
        units.extend(std::iter::repeat_n(0xFDD1, K));
        units.extend("*/".encode_utf16());
        for i in 0..N {
            units.extend("/".encode_utf16());
            units.push(0xD800);
            units.extend(format!("{i}/;").encode_utf16());
        }
        let t0 = std::time::Instant::now();
        let r = compile_eval(
            &shim_text(&units),
            &ep9_bytecode::EvalContext::default(),
            CompileOptions::default(),
        );
        let _ = tx.send((r, t0.elapsed()));
    });
    let (r, took) = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the compile ends in 20 s");
    let u = r.expect("compiles");
    for i in [0, N / 2, N - 1] {
        let i = i.to_string();
        assert!(has_string(&u, &[P::U(0xD800), P::T(&i)]), "{i}");
    }
    assert_eq!(placeholders_left(&u), 0);
    eprintln!("K = {K}, N = {N}: {took:?}");
}

/// The `Function` constructor checks each part on its decoded text (Codex r1 FE-SHIM 4): a
/// lone surrogate and a real U+FDD0 in a regex class range of the body or of a parameter
/// default compile, as in node (`Function("return /[a-\uD800-!]/")().test("!")` is true).
/// Before, (a) and (b) parsed the shim text, where `ESC d800` adds the digits `d800`
/// (`[a-ESC d800-!]` has the reversed range `0-!`) and ESC ESC is two U+FDD0.
#[test]
fn shim_function_constructor_checks_the_decoded_parts() {
    let ctor = |params: &[P<'_>], body: &[P<'_>]| {
        compile_function_ctor(
            ep9_bytecode::FunctionCtorKind::Normal,
            &shim_text(&wtf16(params)),
            &shim_text(&wtf16(body)),
            CompileOptions::default(),
        )
    };
    for (params, body) in [
        (
            vec![],
            vec![P::T("return /[a-"), P::U(0xD800), P::T("-!]/")],
        ),
        (vec![], vec![P::T("return /[a-\u{FDD0}-!]/")]),
        (
            vec![P::T("a = /[a-"), P::U(0xD800), P::T("-!]/")],
            vec![P::T("return a")],
        ),
    ] {
        let r = ctor(&params, &body);
        assert!(r.is_ok(), "{r:?}");
    }
    let u = ctor(&[], &[P::T("return /[a-"), P::U(0xD800), P::T("-!]/")]).expect("compiles");
    assert!(has_string(&u, &[P::T("[a-"), P::U(0xD800), P::T("-!]")]));
    // Out of order with the real code units (DC00 > DBFF), in order as U+FFFD-U+FFFD.
    let r = ctor(
        &[],
        &[
            P::T("return /["),
            P::U(0xDC00),
            P::T("-"),
            P::U(0xDBFF),
            P::T("]/"),
        ],
    );
    assert!(is_syntax(&r), "{r:?}");
}

/// The `Function` constructor reaches the shim through its joined text.
#[test]
fn shim_function_constructor() {
    let body = shim_text(&wtf16(&[
        P::T("// "),
        P::U(0xDABC),
        P::T("\nreturn '"),
        P::U(0xDC01),
        P::T("'"),
    ]));
    let u = compile_function_ctor(
        ep9_bytecode::FunctionCtorKind::Normal,
        "",
        &body,
        CompileOptions::default(),
    )
    .expect("compiles");
    assert!(has_string(&u, &[P::U(0xDC01)]));
}

/// FE-RX-BUFBOUND, FE-RX-BIGQUANT: the regex literal verdicts of node v26.13.0
/// (`x = <literal>;`), with one row where ECMA-262 and node differ. oxc_regular_expression 0.157 takes `\A`, `\z`, `\Z` and `[\c]` in
/// Unicode mode, and refuses a braced quantifier number above 2^53 - 1.
#[test]
fn regex_literals_follow_node_on_escapes_and_big_quantifiers() {
    let cases: &[(&str, bool)] = &[
        (r"/\A/u", false),
        (r"/\z/u", false),
        (r"/\Z/v", false),
        (r"/\A/", true),
        (r"/\z/", true),
        (r"/[\A]/u", false),
        (r"/[\z]/v", false),
        (r"/[\c]/u", false),
        (r"/[\c_]/u", false),
        (r"/[\c1]/u", false),
        (r"/[\c]/", true),
        (r"/[\c_]/", true),
        (r"/[\cA]/u", true),
        (r"/\cA/u", true),
        (r"/\c/u", false),
        (r"/[\q{\z}]/v", false),
        (r"/\\A/u", true),
        (r"/\\c/u", true),
        (r"/[\\c]/u", true),
        (r"/a{9007199254740991}/", true),
        (r"/a{9007199254740992}/", true),
        (r"/a{99999999999999999999}/", true),
        (r"/a{99999999999999999999}/u", true),
        (r"/a{99999999999999999999,}/", true),
        (r"/a{1,99999999999999999999}/", true),
        (r"/a{99999999999999999999,1}/", false),
        // Node takes this one (V8 makes both numbers "infinity", then they are equal).
        // ECMA-262 22.2.1.1 compares the MVs: 10^20 - 1 > 10^20 - 2 is a SyntaxError.
        (r"/a{99999999999999999999,99999999999999999998}/", false),
        (r"/a{99999999999999999998,99999999999999999999}/", true),
        (r"/a{99999999999999999999,99999999999999999999}/", true),
        (r"/a{2,99999999999999999999}/", true),
        (r"/{99999999999999999999}/", false),
        (r"/{99999999999999999999}/u", false),
        (r"/a{99999999999999999999}(/", false),
        (r"/a{99999999999999999999}{1}/", false),
        (r"/\u{99999999999999999999}/", true),
        (r"/\u{99999999999999999999}/u", false),
        (r"/[a{99999999999999999999}]/", true),
        (r"/[[a]{99999999999999999999}]/v", false),
        (r"/a{0099999999999999999999,1}/", false),
        (r"/a{1,0000000000000000000002}/", true),
        (r"/(?<a>x)a{99999999999999999999}\k<b>/", false),
        (r"/a{99999999999999999999}?/", true),
        // Both fixes on one literal (fuzz replay, M7-P5).
        (r"/\A[]{9007199254740992,}/u", false),
        (r"/\z|a{99999999999999999999}/v", false),
        (r"/[\c]a{99999999999999999999}/u", false),
    ];
    for &(literal, node) in cases {
        let src = format!("x = {literal};");
        assert_eq!(ok(&src, SLOPPY), node, "{literal}");
    }
}

/// The new escape error is a SyntaxError at the escape (FE-RX-BUFBOUND).
#[test]
fn regex_unicode_escape_error_has_a_span() {
    let src = r"x = /ab\z/u;";
    let errors = syntax_errors(src, SLOPPY);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let e = errors.first();
    assert!(e.message.contains("Invalid escape sequence"), "{e:?}");
    let at = u32::try_from(src.find(r"\z").expect("escape")).expect("small");
    assert_eq!(
        e.span,
        Some(ByteSpan {
            start: at,
            end: at + 2
        })
    );
}
