//! Regex literal errors after the regex check moved out of oxc_parser (`src/regex_check.rs`).
//!
//! The table holds the results of the base commit (oxc_parser with `parse_regular_expression`
//! on): the same result class in each entry point, and the same regex error spans. The count
//! test (each literal is parsed once) is a unit test in `src/regex_check.rs`, because its
//! counter is test-only crate code.
//!
//! `cargo test -p ep9_frontend --test regex_once`
//! `cargo test -p ep9_frontend --release --test regex_once -- --ignored` (the time bound)

use ep9_bytecode::{EvalContext, FunctionCtorKind, VerifiedUnit};
use ep9_frontend::{
    CodegenError, CompileError, CompileOptions, SourceOpts, Strictness, check_early_errors,
    compile, compile_eval, compile_function_ctor,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Ok,
    Syntax,
    Range,
}

fn class_of(result: &Result<(), CompileError>) -> Class {
    match result {
        Ok(()) => Class::Ok,
        Err(CompileError::Syntax(_)) => Class::Syntax,
        Err(CompileError::Range(_)) => Class::Range,
    }
}

fn class_of_unit(result: &Result<VerifiedUnit, CodegenError>) -> Class {
    match result {
        Ok(_) => Class::Ok,
        Err(CodegenError::Compile(e)) => class_of(&Err(e.clone())),
        Err(other) => panic!("not a compile result: {other:?}"),
    }
}

/// The class in every entry point; all must agree. Returns the error spans of the sloppy
/// script check, sorted (the order can differ from the base commit).
fn run(src: &str) -> (Class, Vec<u32>) {
    let script = check_early_errors(src, SourceOpts::script(Strictness::Sloppy));
    let class = class_of(&script);
    for (name, other) in [
        (
            "strict",
            class_of(&check_early_errors(
                src,
                SourceOpts::script(Strictness::Strict),
            )),
        ),
        (
            "module",
            class_of(&check_early_errors(src, SourceOpts::module())),
        ),
        (
            "compile",
            class_of_unit(&compile(
                src,
                SourceOpts::script(Strictness::Sloppy),
                CompileOptions::default(),
            )),
        ),
        (
            "eval",
            class_of_unit(&compile_eval(
                src,
                &EvalContext::default(),
                CompileOptions::default(),
            )),
        ),
        (
            "function_ctor",
            class_of_unit(&compile_function_ctor(
                FunctionCtorKind::Normal,
                "",
                src,
                CompileOptions::default(),
            )),
        ),
    ] {
        assert_eq!(other, class, "{name}: {src:?}");
    }
    let mut spans: Vec<u32> = match &script {
        Err(CompileError::Syntax(errors)) => errors
            .as_slice()
            .iter()
            .map(|e| e.span.expect("oxc gives a span").start)
            .collect(),
        _ => Vec::new(),
    };
    spans.sort_unstable();
    (class, spans)
}

/// (source, class, sorted error span starts of the sloppy script check), as on the base
/// commit a47c4e6e.
const TABLE: &[(&str, Class, &[u32])] = &[
    // Duplicate group names (triage §7).
    ("/(?<a>x)|(?<a>y)/", Class::Ok, &[]),
    ("/((?<a>x))|((?<a>y))/", Class::Ok, &[]),
    ("/(?<a>x)(?<a>y)/", Class::Syntax, &[4]),
    ("/(?:(?<a>x)|(?<a>y))(?<a>z)/", Class::Syntax, &[7]),
    ("/(?:(?<a>x)|y)(?:(?<a>w)|z)/", Class::Syntax, &[7]),
    // Pattern errors, flags and the u / v modes.
    ("/(/", Class::Syntax, &[1]),
    ("/a/uv", Class::Syntax, &[0, 4]),
    ("/[a&&b]/v", Class::Ok, &[]),
    ("/[a&&&b]/v", Class::Syntax, &[5]),
    ("/\\p{L}/u", Class::Ok, &[]),
    ("/\\p{L}/", Class::Ok, &[]),
    ("/\\p{Foo}/u", Class::Syntax, &[4]),
    ("/{/u", Class::Syntax, &[1]),
    ("/{/", Class::Ok, &[]),
    ("/(?<a>.)\\k<b>/", Class::Syntax, &[8]),
    ("/\\k<b>/", Class::Ok, &[]),
    ("/(?<\u{1d49c}>.)/", Class::Ok, &[]),
    ("/\\u{1F600}/u", Class::Ok, &[]),
    ("/[\\u{1F600}-\\u{1F601}]/v", Class::Ok, &[]),
    ("/(?i:a)/", Class::Ok, &[]),
    ("/(?i-i:a)/", Class::Syntax, &[3]),
    ("/a{2,1}/", Class::Syntax, &[2]),
    ("/(?:)/", Class::Ok, &[]),
    // Span offsets: `/=`, non-ASCII text before the literal, several literals.
    ("x = /=(/", Class::Syntax, &[6]),
    ("\"\u{e9}\";\n  /(?<a>x)(?<a>y)/g", Class::Syntax, &[12]),
    ("/(/; /)/; /[z-a]/", Class::Syntax, &[1, 6, 13]),
    // Literals that oxc parses in a rewound arrow head or cover grammar.
    ("(a = /(?<n>x)(?<n>y)/) => a", Class::Syntax, &[9]),
    ("({a = /(/} = {})", Class::Syntax, &[7]),
    ("async (a = /(/) => a", Class::Syntax, &[12]),
    // A regex error next to a recoverable parser error.
    ("new.target; /(/", Class::Syntax, &[0, 13]),
    // Lexer errors stay fatal parser errors (the pattern is not parsed).
    ("/a/gg", Class::Syntax, &[0, 4]),
    ("/a/x", Class::Syntax, &[0, 3]),
    ("/[/", Class::Syntax, &[0, 0]),
];

#[test]
fn early_errors_match_the_base_commit() {
    for &(src, class, spans) in TABLE {
        assert_eq!(run(src), (class, spans.to_vec()), "{src:?}");
    }
}

/// rx-dupnames (source, class, sorted error span starts): two groups of one name that can
/// both take part are an early error (MightBothParticipate, ECMA-262 22.2.1.1). oxc 0.155.0
/// accepted the first 5 rows; oxc 0.157.0 and node v26.13.0 reject them. The span is the
/// name of the earlier group. The other rows agree with node on both oxc versions.
const DUPNAMES: &[(&str, Class, &[u32])] = &[
    ("/(?:(?<a>x)|y)(?:z|(?<a>w))/", Class::Syntax, &[7]),
    ("/(?<n>a|(?<n>b))/", Class::Syntax, &[4]),
    ("/(?:a|(?<n>x))(?<n>y)/", Class::Syntax, &[9]),
    ("/(?<a>x)(?:|(?<a>y))/", Class::Syntax, &[4]),
    ("/(?<a>x|(?<a>y))/", Class::Syntax, &[4]),
    // The same names in alternatives that cannot both take part stay valid (node agrees).
    ("/(?:(?<a>x)|(?<a>y))|(?<a>z)/", Class::Ok, &[]),
    ("/(?<a>(?<b>x)|(?<b>y))|(?<b>z)/", Class::Ok, &[]),
    ("/(?<a>x)|[(](?<a>y)/", Class::Ok, &[]),
    ("/(?i:(?<a>x))|(?<a>y)/", Class::Ok, &[]),
    ("/(?i:(?<a>x)|y)(?<a>z)/", Class::Syntax, &[8]),
    ("/[\\q{a|b}](?<a>y)|(?<a>z)/v", Class::Ok, &[]),
    ("/[\\q{a|b}](?<a>y)(?<a>z)/v", Class::Syntax, &[13]),
];

#[test]
fn duplicate_names_that_can_both_take_part_are_syntax_errors() {
    for &(src, class, spans) in DUPNAMES {
        assert_eq!(run(src), (class, spans.to_vec()), "{src:?}");
    }
}

#[test]
fn a_fatal_parser_error_hides_earlier_regex_errors() {
    // The one known change: after a fatal parser error oxc gives no tokens, so the regex
    // errors in front of it are not reported. Still a SyntaxError. The base commit gave
    // [1, 5] for both (the regex error first).
    assert_eq!(run("/(/; 1 = 2;"), (Class::Syntax, vec![5]));
    assert_eq!(run("/(/; }"), (Class::Syntax, vec![5]));
}

#[test]
fn a_regex_error_inside_50_arrow_heads_is_a_syntax_error() {
    let depth = 50;
    let src = format!(
        "{}/(?<a>x)(?<a>y)/{}",
        "(a=".repeat(depth),
        ")".repeat(depth)
    );
    // The pattern starts after 50 x "(a=" and the `/`. The label is on the name of the
    // first group (`a`, 3 bytes into the pattern), as on the base commit (154).
    let pattern = 3 * 50 + 1;
    assert_eq!(run(&src), (Class::Syntax, vec![pattern + 3]));
    let ok = format!(
        "{}/(?<a>x)|(?<a>y)/{}",
        "(a=".repeat(depth),
        ")".repeat(depth)
    );
    assert_eq!(run(&ok), (Class::Ok, vec![]));
}

#[test]
fn a_regex_error_below_the_depth_cap_is_a_syntax_error() {
    // A member chain deeper than AST_CAP with the bad literal at its bottom: the regex error
    // comes before the depth guard, as on the base commit. Without it, a RangeError.
    let chain = ".a".repeat(ep9_frontend::AST_CAP as usize + 1);
    assert_eq!(run(&format!("/(/{chain}")), (Class::Syntax, vec![1]));
    assert_eq!(run(&format!("/(?:)/{chain}")), (Class::Range, vec![]));
}

/// The n = 4096 input of the triage curve (`gen/curve.py`, without the 2 context bytes):
/// d = 255 x `(a=`, r = 511 groups, g = 256 alternatives `(?<a>x)|`.
fn curve_4096() -> String {
    let (d, r, g) = (255, 511, 256);
    format!(
        "{}/{}{}{}/{}",
        "(a=".repeat(d),
        "(".repeat(r),
        "(?<a>x)|".repeat(g),
        ")".repeat(r),
        ")".repeat(d)
    )
}

#[test]
#[ignore = "measures time; run in release"]
fn the_triage_input_compiles_fast() {
    // Before the fix: 1.2 s native release (31 s in the ASan fuzz build). The bound is loose.
    let src = curve_4096();
    let start = std::time::Instant::now();
    let r = compile_eval(&src, &EvalContext::default(), CompileOptions::default());
    let took = start.elapsed();
    assert!(r.is_ok(), "{:?}", r.err());
    assert!(took.as_secs_f64() < 0.1, "compile_eval took {took:?}");
}
