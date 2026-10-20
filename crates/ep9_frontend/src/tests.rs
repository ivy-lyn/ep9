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
