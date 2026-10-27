//! The `Function` constructor family (ECMA-262 CreateDynamicFunction, §20.2.1.1.1):
//! [`compile_function_ctor`] turns the parameter text and the body text into a
//! [`VerifiedUnit`] whose function 0 returns the new function. The VM calls it through its
//! compiler hook (`IsolateConfig::compiler`, C2) after it has converted the arguments to
//! strings; ep9_vm has no edge to this crate.
//!
//! oxc has no entry point that parses a string as FormalParameters or as FunctionBody alone
//! (docs/upstream/oxc-function-constructor.md), and one parse of the joined text accepts inputs
//! that must throw (`new Function("/*", "*/){")`). So the parameters and the body are each
//! parsed inside a wrapper whose shape is then checked:
//!
//! ```text
//! caller thread: prescan (layer 0) of each of the 3 texts
//! front-end worker (one job; the nested compile_eval runs inline):
//!   (a) "(<prefix> anonymous(" + P + "\n) {\n})"        parse, layer 2; one statement: a
//!       parenthesized function expression whose parameter list ends at the wrapper's `)`  --> Syntax
//!   (b) "(<prefix> anonymous(\n) {\n" + body + "\n})"   parse, layer 2; the same shape, and
//!       the body ends at the wrapper's `}`                                                 --> Syntax
//!   (c) compile_eval("({anonymous: <prefix>(" + P + "\n) {\n" + body + "\n}}).anonymous",
//!                    indirect sloppy EvalContext)       every early error, scope pass, BytecodeGen
//!   --> VerifiedUnit (CodeKind::Eval): function 0 returns the closure (function 1, "anonymous")
//! ```
//!
//! # Decisions
//!
//! | # | Decision | Why |
//! |---|---|---|
//! | F1 | Three parses: (a) and (b) each check one part in a wrapper, (c) compiles the joined text | CreateDynamicFunction parses P with the FormalParameters goal and the body with the FunctionBody goal, each on its own, then the joined text. Only the wrapper's shape (one function, the part ends at the wrapper's own `)` or `}`) shows that a part did not leave its place |
//! | F2 | (a) and (b) run the parser and the depth layers only, not oxc_semantic | (c) has the same parameters and the same body in the same place, so oxc_semantic on (c) reports every early error of (a) and (b); a token cannot cross a part's end in (c) unless it does in (a) or (b) (each part is followed by the same `\n`) |
//! | F3 | (c) goes through [`compile_eval`](crate::compile_eval) with `EvalContext::default()` (indirect, sloppy): a `CodeKind::Eval` unit whose function 0 returns the completion value (E5), run by the VM as global code (D8) | A Script unit's function 0 ends in `LdaUndefined; Return`; indirect eval is global code over the global env, which is the env of CreateDynamicFunction |
//! | F4 | (c) wraps the function in `({anonymous: ...}).anonymous`, not `(function anonymous(...) {...})` | A named function expression binds its own name inside its body; CreateDynamicFunction does not (OrdinaryFunctionCreate over the global env, then SetFunctionName(F, "anonymous")). node 24: `new Function("return typeof anonymous")()` is `"undefined"`. The property value gets the name `anonymous` by NamedEvaluation and stays an ordinary function expression (a constructor with a `prototype`, no home object) |
//! | F5 | The Async and AsyncGenerator kinds run the same checks; BytecodeGen then answers `Unsupported` ("async functions"; "generator functions" for an async generator, GEN6) | A text that is not valid for the kind is a SyntaxError today; a valid one is "not built yet", never a SyntaxError (D7) |
//! | F6 | Error messages and positions refer to the wrapper text | The VM throws the message only (`CompileFailure`) |

use ep9_bytecode::{EvalContext, FunctionCtorKind, VerifiedUnit};
use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, Function, Program, Statement};

use crate::bytecodegen::{CodegenError, CompileOptions};
use crate::{
    ByteSpan, CompileError, DepthLayer, EarlyErrors, LineMap, PRESCAN_CAP, RangeError,
    RangeErrorKind, SourceOpts, Strictness, SyntaxError,
};

/// Compile a function of `kind` from the parameter text `params` (the arguments joined with
/// `,`) and the body text `body` (CreateDynamicFunction).
///
/// The result is a [`CodeKind::Eval`](ep9_bytecode::CodeKind) unit. The VM runs its function 0
/// as indirect eval code (no context, the global this as receiver); function 0 returns the new
/// function, function 1 of the unit, named `anonymous`, closed over the global env. The VM then
/// sets the function's `[[Prototype]]` (GetPrototypeFromConstructor).
///
/// # Errors
///
/// See [`CodegenError`]: a SyntaxError when `params` alone is not a parameter list, when `body`
/// alone is not a function body, or for an early error of the whole function; a RangeError for
/// nesting above the depth caps; [`CodegenError::Unsupported`] for the async kinds and for every
/// construct that BytecodeGen v0 does not support.
pub fn compile_function_ctor(
    kind: FunctionCtorKind,
    params: &str,
    body: &str,
    options: CompileOptions,
) -> Result<VerifiedUnit, CodegenError> {
    let texts = Texts::new(kind, params, body);
    // Layer 0 on the caller's thread, in the spec order (parameters, body, whole).
    for text in [&texts.params, &texts.body, &texts.joined] {
        let estimate = crate::prescan::estimate(text.as_bytes(), false);
        if estimate.depth > PRESCAN_CAP {
            return Err(too_deep(DepthLayer::PreScan, text, estimate.at));
        }
    }
    let job = move || {
        check_part(&texts.params, Part::Params)?;
        check_part(&texts.body, Part::Body)?;
        // On the worker, compile_eval runs inline (worker::run).
        crate::compile_eval(&texts.joined, &EvalContext::default(), options)
    };
    match crate::worker::run(crate::stack::WORKER_STACK_BYTES, job) {
        Ok(result) => result,
        Err(spawn) => Err(CompileError::Range(RangeError {
            kind: RangeErrorKind::StackUnavailable,
            message: spawn.message,
            span: None,
            position: None,
        })
        .into()),
    }
}

/// The three source texts (F1, F4).
struct Texts {
    /// (a): the parameters in an empty function.
    params: String,
    /// (b): the body in a function without parameters.
    body: String,
    /// (c): the joined text that is compiled.
    joined: String,
}

impl Texts {
    fn new(kind: FunctionCtorKind, params: &str, body: &str) -> Self {
        let prefix = kind.prefix();
        Self {
            params: format!("({prefix} anonymous({params}\n) {{\n}})"),
            body: format!("({prefix} anonymous(\n) {{\n{body}\n}})"),
            joined: format!("({{anonymous: {prefix}({params}\n) {{\n{body}\n}}}}).anonymous"),
        }
    }
}

/// Which part a wrapper text checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Params,
    Body,
}

/// Parse `text`, wrapper (a) or (b), and check its shape (F1, F2). On the worker.
fn check_part(text: &str, part: Part) -> Result<(), CodegenError> {
    let allocator = Allocator::default();
    let program = crate::parse(&allocator, text, SourceOpts::script(Strictness::Sloppy))?;
    // The wrapper ends in `\n) {\n})` (a) or `\n})` (b): the part must end right before it.
    let ok = the_function(&program).is_some_and(|f| match part {
        // The parameter list's span holds its parentheses: it ends after the wrapper's `)`,
        // which is followed by ` {\n})` (5 bytes).
        Part::Params => f.params.span.end as usize + 5 == text.len(),
        // The body's span holds its braces: it ends after the wrapper's `}`, followed by `)`.
        Part::Body => f
            .body
            .as_ref()
            .is_some_and(|b| b.span.end as usize + 1 == text.len()),
    });
    if ok {
        return Ok(());
    }
    let message = match part {
        Part::Params => "Arg string terminates parameters early",
        Part::Body => "the Function body text closes the function early",
    };
    Err(CodegenError::Compile(CompileError::Syntax(EarlyErrors {
        errors: vec![SyntaxError {
            message: message.to_owned(),
            span: Some(ByteSpan { start: 0, end: 0 }),
            position: Some(LineMap::new(text.as_bytes()).position(0)),
        }],
    })))
}

/// The function of a wrapper text: the program is one expression statement that is one
/// parenthesized function expression, with no directive and no hashbang.
fn the_function<'p, 'a>(program: &'p Program<'a>) -> Option<&'p Function<'a>> {
    if program.hashbang.is_some() || !program.directives.is_empty() || program.body.len() != 1 {
        return None;
    }
    let Statement::ExpressionStatement(statement) = &program.body[0] else {
        return None;
    };
    let Expression::ParenthesizedExpression(paren) = &statement.expression else {
        return None;
    };
    match &paren.expression {
        Expression::FunctionExpression(f) => Some(f),
        _ => None,
    }
}

fn too_deep(layer: DepthLayer, source: &str, at: u32) -> CodegenError {
    CodegenError::Compile(CompileError::Range(RangeError::too_deep(
        layer,
        source,
        Some(at),
    )))
}

#[cfg(test)]
mod tests {
    use ep9_bytecode::{
        CodeKind, FunctionFlags, FunctionId, FunctionKind, UnitString, disassemble_function,
    };

    use super::*;
    use crate::AST_CAP;

    fn compile(
        kind: FunctionCtorKind,
        params: &str,
        body: &str,
    ) -> Result<VerifiedUnit, CodegenError> {
        compile_function_ctor(kind, params, body, CompileOptions::default())
    }

    /// The result as the cases file writes it: `ok`, `SyntaxError`, `RangeError`,
    /// `unsupported: <feature>` or `internal: <message>`.
    fn outcome(r: &Result<VerifiedUnit, CodegenError>) -> String {
        match r {
            Ok(_) => "ok".to_owned(),
            Err(CodegenError::Compile(CompileError::Syntax(_))) => "SyntaxError".to_owned(),
            Err(CodegenError::Compile(CompileError::Range(_)) | CodegenError::Limit(_)) => {
                "RangeError".to_owned()
            }
            Err(CodegenError::Unsupported(u)) => format!("unsupported: {}", u.feature),
            Err(CodegenError::Internal(m)) => format!("internal: {m}"),
        }
    }

    /// The D8 unit contract (EVAL-VM builds on it): an Eval unit whose function 0 makes the
    /// closure of function 1 (`anonymous`) and returns it, and declares nothing.
    #[test]
    fn the_unit_contract() {
        for (kind, body, generator) in [
            (FunctionCtorKind::Normal, "return a + b", false),
            (FunctionCtorKind::Generator, "yield a + b", true),
        ] {
            let unit = compile(kind, "a,b", body).unwrap();
            let u = unit.unit();
            assert_eq!(u.header.code_kind, CodeKind::Eval);
            assert_eq!(u.functions.len(), 2);
            let f0 = &u.functions[0];
            let f1 = &u.functions[1];
            assert_eq!(f0.kind, FunctionKind::Eval);
            assert!(!f0.flags.contains(FunctionFlags::STRICT));
            assert_eq!(f1.kind, FunctionKind::Normal);
            assert_eq!(f1.parent, Some(FunctionId(0)));
            assert_eq!(f1.flags.contains(FunctionFlags::GENERATOR), generator);
            assert_eq!((f1.parameter_count, f1.length), (2, 2));
            let name = f1.name.map(|s| u.string(s).clone());
            assert_eq!(name, Some(UnitString::new("anonymous")));

            // Function 0: the closure goes into the object, is read back and returned.
            let d0 = disassemble_function(u, FunctionId(0));
            let code: Vec<&str> = d0
                .lines()
                .take_while(|l| !l.trim_start().starts_with("constants"))
                .filter_map(|l| {
                    l.split_whitespace().find(|w| {
                        w.starts_with(|c: char| c.is_ascii_uppercase())
                            && w.chars().all(|c| c.is_ascii_alphanumeric())
                    })
                })
                .collect();
            assert_eq!(
                code,
                [
                    "CreateObjectLiteral",
                    "Star",
                    "CreateClosure",
                    "DefineNamedOwnProperty",
                    "Ldar",
                    "Star",
                    "GetNamedProperty",
                    "Star",
                    "Ldar",
                    "Return"
                ],
                "{d0}"
            );
            assert!(d0.contains("Function f1"), "{d0}");
            assert!(!d0.contains("DeclareEval"), "{d0}");
        }
    }

    /// F4: the body does not see a binding `anonymous` (CreateDynamicFunction makes none).
    #[test]
    fn anonymous_is_not_bound_in_the_body() {
        let unit = compile(FunctionCtorKind::Normal, "", "return anonymous").unwrap();
        let d1 = disassemble_function(unit.unit(), FunctionId(1));
        assert!(d1.contains("LdaGlobal"), "{d1}");
        assert!(!d1.contains("LdaCurrentFunction"), "{d1}");
    }

    /// docs/upstream/oxc-function-constructor.md: one parse of the joined text accepts both.
    #[test]
    fn the_upstream_cases_are_syntax_errors() {
        let params = compile(FunctionCtorKind::Normal, "/*", "*/){");
        let body = compile(FunctionCtorKind::Normal, "", "}); (function(){");
        assert_eq!(outcome(&params), "SyntaxError");
        assert_eq!(outcome(&body), "SyntaxError");
        let Err(CodegenError::Compile(CompileError::Syntax(e))) = body else {
            unreachable!()
        };
        assert_eq!(
            e.first().message,
            "the Function body text closes the function early"
        );
        let Err(CodegenError::Compile(CompileError::Syntax(e))) =
            compile(FunctionCtorKind::Normal, "a){}, function(b", "")
        else {
            panic!("a shape error")
        };
        assert_eq!(e.first().message, "Arg string terminates parameters early");
    }

    /// F5: the async kinds check the text, then answer Unsupported.
    #[test]
    fn async_kinds_are_unsupported_after_the_checks() {
        let r = compile(FunctionCtorKind::Async, "a", "return await a");
        assert_eq!(outcome(&r), "unsupported: async functions");
        let r = compile(FunctionCtorKind::AsyncGenerator, "", "yield await 1");
        assert_eq!(outcome(&r), "unsupported: generator functions");
        let r = compile(FunctionCtorKind::Async, "", "}); (async function(){");
        assert_eq!(outcome(&r), "SyntaxError");
    }

    /// Each part and the joined text are checked against the depth caps (layer 0 on the
    /// caller's thread, layer 2 on the worker).
    #[test]
    fn deep_parts_are_range_errors() {
        let deep = "[".repeat(1_000_000);
        for (params, body) in [(deep.as_str(), ""), ("", deep.as_str())] {
            let r = compile(FunctionCtorKind::Normal, params, body);
            let Err(CodegenError::Compile(CompileError::Range(e))) = r else {
                panic!("{}", outcome(&r))
            };
            assert_eq!(e.kind, RangeErrorKind::TooDeep(DepthLayer::PreScan));
        }
        // Below the pre-scan cap, above the AST cap: layer 2 on the worker, in part (a) or (b).
        let n = AST_CAP as usize + 1_000;
        let nested = format!("{}{}", "[".repeat(n), "]".repeat(n));
        for (params, body) in [
            (format!("a = {nested}"), String::new()),
            (String::new(), nested),
        ] {
            let r = compile(FunctionCtorKind::Normal, &params, &body);
            let Err(CodegenError::Compile(CompileError::Range(e))) = r else {
                panic!("{}", outcome(&r))
            };
            assert_eq!(e.kind, RangeErrorKind::TooDeep(DepthLayer::Ast));
        }
    }

    /// `tests/function_ctor_cases.txt`: the compile result matches node (the file's 4th
    /// column), or the ep9 column where M0 differs.
    #[test]
    fn the_cases_match_node() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/function_ctor_cases.txt");
        let text = std::fs::read_to_string(path).unwrap();
        let mut bad = Vec::new();
        let mut n = 0;
        for (i, line) in text.lines().enumerate() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            assert!(cols.len() == 4 || cols.len() == 5, "line {}: {line}", i + 1);
            let kind = match cols[0] {
                "normal" => FunctionCtorKind::Normal,
                "generator" => FunctionCtorKind::Generator,
                "async" => FunctionCtorKind::Async,
                "asyncgen" => FunctionCtorKind::AsyncGenerator,
                k => panic!("line {}: kind {k}", i + 1),
            };
            let want = cols
                .get(4)
                .map_or(cols[3], |c| c.strip_prefix("ep9=").expect("an ep9= column"));
            let got = outcome(&compile(kind, &json_string(cols[1]), &json_string(cols[2])));
            n += 1;
            if got != want {
                bad.push(format!("line {}: ep9 {got}, want {want}: {line}", i + 1));
            }
        }
        assert!(n >= 30, "{n} cases");
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    /// A JSON string literal (the escapes `json.dumps` writes with `ensure_ascii`).
    fn json_string(lit: &str) -> String {
        let inner = lit
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .expect("a JSON string");
        let mut units: Vec<u16> = Vec::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                let mut buf = [0; 2];
                units.extend_from_slice(c.encode_utf16(&mut buf));
                continue;
            }
            let unit = match chars.next().expect("an escape") {
                'n' => 0x0a,
                't' => 0x09,
                'r' => 0x0d,
                'b' => 0x08,
                'f' => 0x0c,
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    u16::from_str_radix(&hex, 16).expect("4 hex digits")
                }
                c => u16::try_from(u32::from(c)).expect("an ASCII escape"),
            };
            units.push(unit);
        }
        String::from_utf16(&units).expect("no lone surrogate")
    }
}
