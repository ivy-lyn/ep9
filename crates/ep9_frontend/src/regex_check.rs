//! The regex literal check: each regex literal is parsed once, after the oxc parse.
//!
//! ```text
//! oxc_parser: parse_regular_expression off, tokens on --> AST, tokens, diagnostics
//!   v
//! check: for each RegExp token of the final token list
//!   oxc_regular_expression::LiteralParser (the same pattern text, flags text and span
//!   offsets as oxc_parser 0.155 parse_literal_regexp) --> an error goes into diagnostics
//!   v
//! the callers read the diagnostics as before (lib.rs parse, eval.rs on_worker)
//! ```
//!
//! Why (fuzz triage of the eval_context timeout bb32e719, Fix 1): with
//! `parse_regular_expression` on, oxc_parser parses the pattern each time that it parses the
//! literal. It parses the text inside a failed arrow head (`(a=` ...) one more time, so a
//! literal inside d such heads is parsed d+1 times. The duplicate group name check of
//! oxc_regular_expression 0.155 is quadratic in the number of groups, so the time grows with
//! d x g^2. Here the pattern is parsed once for each literal.
//!
//! Why the token list and not an AST walk:
//! - A parser rewind (arrow head, lookahead) cuts the token list where it cuts the parser
//!   errors. So the final list has one RegExp token for each literal of the last parse pass,
//!   and the set of regex errors is the same as with `parse_regular_expression` on.
//! - It is a flat loop. An AST walk must stop at the depth cap (a member chain `/(/.a.a...`
//!   deeper than `AST_CAP`), and then a regex error below the cap is a RangeError, not a
//!   SyntaxError.
//! - The regex parser recurses (`prescan::REGEX_WEIGHT`). Here it runs near the base of the
//!   worker stack, not at the parser's depth, so the stack budget of `stack.rs` still holds.
//!
//! Limits (the result is a SyntaxError in each case, as before):
//! - After a fatal parse error, oxc gives an empty token list (and an empty AST). Then only
//!   the parser errors are reported. Before, the regex errors in front of the fatal error
//!   were reported too.
//! - The regex errors come after all oxc_parser errors. So the first error (the thrown
//!   message) can change: for `x = /a/uv;` it is now the parser's `u` and `v` flags error.
//! - oxc_parser adds "Flow is not supported" (a first comment with `@flow`) only when it
//!   found an error itself. A regex error does not count now. So `/* @flow */ /(/;` gives the
//!   regex error and not the Flow error.

use oxc_allocator::Allocator;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::{Kind, Token};
use oxc_regular_expression::{LiteralParser, Options};

#[cfg(test)]
thread_local! {
    /// Test only: the number of `LiteralParser` runs on this thread.
    static LITERAL_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test only: the number of `LiteralParser` runs on this thread so far.
#[cfg(test)]
pub(crate) fn literal_parses() -> usize {
    LITERAL_PARSES.with(std::cell::Cell::get)
}

/// Parse the pattern and flags of each RegExp token in `tokens` once, and push each error
/// into `diagnostics`.
///
/// `tokens` must be the final token list of an oxc parse of `source` (`TokensParserConfig`).
pub(crate) fn check(source: &str, tokens: &[Token], diagnostics: &mut Vec<OxcDiagnostic>) {
    // One scratch arena for all literals: the parsed patterns are not kept.
    let mut scratch: Option<Allocator> = None;
    for token in tokens.iter().filter(|t| t.kind() == Kind::RegExp) {
        let allocator = scratch.get_or_insert_with(Allocator::default);
        if let Some(error) = check_literal(allocator, source, token.start(), token.end()) {
            diagnostics.push(error);
        }
        allocator.reset();
    }
}

/// Parse the regex literal at `source[start..end]` (`/pattern/flags`). Returns its error.
fn check_literal(
    allocator: &Allocator,
    source: &str,
    start: u32,
    end: u32,
) -> Option<OxcDiagnostic> {
    let literal = &source[start as usize..end as usize];
    debug_assert!(literal.starts_with('/'));
    // The flags are ASCII letters, digits, `$` and `_` (oxc lexer read_regex), so the last `/`
    // closes the pattern. The opening `/` is at index 0.
    let close = literal.rfind('/').filter(|&i| i > 0)?;
    let pattern_end = start + u32::try_from(close).ok()?;
    // As oxc_parser parse_literal_regexp: the pattern starts after the opening `/`, the flags
    // after the closing `/`.
    let pattern_start = start + 1;
    let flags_start = pattern_end + 1;
    let pattern = &source[pattern_start as usize..pattern_end as usize];
    let flags = &source[flags_start as usize..end as usize];
    #[cfg(test)]
    LITERAL_PARSES.with(|n| n.set(n.get() + 1));
    let options = Options {
        pattern_span_offset: pattern_start,
        flags_span_offset: flags_start,
    };
    LiteralParser::new(allocator, pattern, Some(flags), options)
        .parse()
        .err()
}

#[cfg(test)]
mod tests {
    use ep9_bytecode::{EvalContext, FunctionCtorKind};
    use oxc_ast::ast::RegExpLiteral;
    use oxc_ast_visit::Visit;
    use oxc_parser::config::TokensParserConfig;
    use oxc_parser::{ParseOptions, Parser};
    use oxc_span::{SourceType, Span};

    use super::*;
    use crate::{
        CodegenError, CompileError, CompileOptions, SourceOpts, Strictness, compile, compile_eval,
        compile_function_ctor,
    };

    /// The shape of the fuzz triage (section 7): 200 x `(a=` around one regex literal with
    /// 200 nested groups and 150 alternatives that all name a group `a`.
    fn triage_shape() -> String {
        let groups = vec!["(?<a>x)"; 150].join("|");
        format!(
            "{}/{}{groups}{}/{}",
            "(a=".repeat(200),
            "(".repeat(200),
            ")".repeat(200),
            ")".repeat(200)
        )
    }

    /// Run `f` on the front-end worker and return the `LiteralParser` runs it made. The
    /// counter is per thread, and the compile entry points run inline on the worker.
    fn parses_during(f: impl FnOnce() + Send + 'static) -> usize {
        crate::worker::run(crate::stack::WORKER_STACK_BYTES, move || {
            let before = literal_parses();
            f();
            literal_parses() - before
        })
        .expect("worker")
    }

    fn is_ok<T>(result: &Result<T, CodegenError>) -> bool {
        result.is_ok()
    }

    #[test]
    fn each_literal_is_parsed_once_in_eval() {
        let src = triage_shape();
        let n = parses_during(move || {
            let r = compile_eval(&src, &EvalContext::default(), CompileOptions::default());
            assert!(is_ok(&r), "{:?}", r.err());
        });
        // Before the fix, oxc parsed this pattern 201 times (inside oxc, not counted here).
        assert_eq!(n, 1);
    }

    #[test]
    fn each_literal_is_parsed_once_in_a_script() {
        let src = triage_shape();
        let n = parses_during(move || {
            let r = compile(
                &src,
                SourceOpts::script(Strictness::Sloppy),
                CompileOptions::default(),
            );
            assert!(is_ok(&r), "{:?}", r.err());
        });
        assert_eq!(n, 1);
    }

    #[test]
    fn each_literal_is_parsed_once_per_function_ctor_text() {
        let src = triage_shape();
        let n = parses_during(move || {
            let r = compile_function_ctor(
                FunctionCtorKind::Normal,
                "",
                &src,
                CompileOptions::default(),
            );
            assert!(is_ok(&r), "{:?}", r.err());
        });
        // The body is in two texts: wrapper (b) and the joined text (c). The parameter text
        // (a) has no literal.
        assert_eq!(n, 2);
    }

    #[test]
    fn the_count_is_one_per_literal() {
        let n = parses_during(|| {
            let r = compile_eval(
                "(a=(a=/x/)); /y/g; [/z/u, /(?<n>.)/v]",
                &EvalContext::default(),
                CompileOptions::default(),
            );
            assert!(is_ok(&r), "{:?}", r.err());
        });
        assert_eq!(n, 4);
    }

    #[test]
    fn a_regex_error_in_a_deep_chain_stays_a_syntax_error() {
        // The literal is below AST_CAP member levels. The AST is too deep (layer 2), but the
        // regex error comes first, as with oxc's own regex parse.
        let src = format!("/(/{}", ".a".repeat(crate::AST_CAP as usize + 1));
        let r = compile_eval(&src, &EvalContext::default(), CompileOptions::default());
        assert!(
            matches!(r, Err(CodegenError::Compile(CompileError::Syntax(_)))),
            "{:?}",
            r.err()
        );
        let r = crate::check_early_errors(&src, SourceOpts::script(Strictness::Sloppy));
        assert!(matches!(r, Err(CompileError::Syntax(_))), "{r:?}");
    }

    /// The RegExpLiteral spans of the AST.
    #[derive(Default)]
    struct Literals(Vec<Span>);

    impl<'a> Visit<'a> for Literals {
        fn visit_reg_exp_literal(&mut self, it: &RegExpLiteral<'a>) {
            self.0.push(it.span);
        }
    }

    #[test]
    fn the_regexp_tokens_are_the_regex_literals_of_the_ast() {
        // Rewinds (arrow heads, `async` arrows, object patterns), `/=`, classes with `/`,
        // escapes and division next to literals.
        for src in [
            "(a=(a=(a=/x/)))",
            "(a = /x/) => a; (b = /y/)",
            "async (a = /x/) => a; async(/y/)",
            "({a = /x/} = {}); ({b: /y/})",
            "x = /=/; y /= 2; z = a / b / c",
            "/[/]/; /\\//; /a/g.source / 2",
            "for (const r of [/a/, /b/]) r; `${/c/}`",
            "class C { static #r = /d/; m(x = /e/) { return x; } }",
        ] {
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, src, SourceType::script())
                .with_options(ParseOptions::default())
                .with_config(TokensParserConfig)
                .parse();
            assert!(!parsed.diagnostics.has_errors(), "{src}");
            let mut ast = Literals::default();
            ast.visit_program(&parsed.program);
            let tokens: Vec<Span> = parsed
                .tokens
                .iter()
                .filter(|t| t.kind() == Kind::RegExp)
                .map(Token::span)
                .collect();
            assert_eq!(tokens, ast.0, "{src}");
        }
    }

    #[test]
    fn spans_and_flags_match_oxc_parser() {
        // For each literal, the diagnostics of check() equal those of oxc_parser with
        // parse_regular_expression on: the same message, the same labels (span offsets), the
        // same flags handling (`u`, `v`, both).
        for src in [
            "/(?<a>x)(?<a>y)/",
            "x = /=(/",
            "\"\u{e9}\";\n  /(?<a>x)(?<a>y)/g",
            "/a/uv",
            "/[a&&&b]/v",
            "/\\p{Foo}/u",
            "/{/u",
            "/{/",
            "/(?<a>.)\\k<b>/",
            "/(?i-i:a)/",
            "/a{2,1}/",
            "/(/; /)/; /[z-a]/",
            "/\\u{1F600}/u; /\\u{1F600}/",
            "/[\\q{abc}]/v; /[\\q{abc}]/u",
        ] {
            let old_allocator = Allocator::default();
            let old = Parser::new(&old_allocator, src, SourceType::script())
                .with_options(ParseOptions {
                    parse_regular_expression: true,
                    ..ParseOptions::default()
                })
                .parse();
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, src, SourceType::script())
                .with_config(TokensParserConfig)
                .parse();
            let mut new: Vec<OxcDiagnostic> = parsed.diagnostics.errors().cloned().collect();
            check(src, &parsed.tokens, &mut new);
            let render = |d: &OxcDiagnostic| {
                let labels: Vec<(u32, u32)> =
                    d.labels.iter().map(|l| (l.offset(), l.len())).collect();
                format!("{} {labels:?}", d.message)
            };
            let mut want: Vec<String> = old.diagnostics.errors().map(render).collect();
            let mut got: Vec<String> = new.iter().map(render).collect();
            want.sort();
            got.sort();
            assert_eq!(got, want, "{src}");
        }
    }
}
