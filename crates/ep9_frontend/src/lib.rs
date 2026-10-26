//! The front end and the only crate that imports oxc: parse, semantic, depth guard, scope pass,
//! `BytecodeGen` with TS lowering, hint join from `.ep9h`, lone-surrogate shim and L1 cache.
//!
//! F1 (M0 week 3) pipeline. Everything after the pre-scan runs on the sized-stack worker:
//!
//! ```text
//! source --layer 0: prescan::estimate--> estimate > PRESCAN_CAP? --> Err(Range)
//!   | copy the source; hop to the front-end worker (stack sized for the caps, stack.rs)
//!   v
//! oxc_parser (+ regex validation) ------------------- errors? --> Err(Syntax)
//!   v strict script? insert a "use strict" directive into the AST
//! layer 2: depth::check_ast_depth ------------------ depth > AST_CAP? --> Err(Range)
//!   v
//! oxc_semantic syntax checks ------------------------ errors? --> Err(Syntax)
//!   v
//! scope pass v0 (scope::analyze, layer 3 checks) --> ScopeAnalysis
//!   v
//! BytecodeGen v0 (bytecodegen::compile) ------------> VerifiedUnit (ep9_bytecode ABI 5)
//! ```
//!
//! - No oxc type leaves this crate (H1). Errors are plain [`SyntaxError`] / [`RangeError`] values.
//! - The source type is always set explicitly: Script or Module goal, JavaScript, no JSX.
//! - Positions are byte offsets; [`LineMap`] turns them into line and UTF-16 column.

use std::fmt;

use oxc_allocator::Allocator;
use oxc_ast::ast::{Directive, Program, StringLiteral};
use oxc_ast::builder::AstBuilder;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::{SPAN, SourceType};

mod bytecodegen;
mod depth;
mod eval;
mod function_ctor;
mod position;
mod prescan;
pub mod scope;
mod stack;
mod strict_filter;
mod worker;

pub use bytecodegen::{
    CodegenError, CompileOptions, Determinism, Unsupported, check_determinism, compile,
    compile_module, compile_script,
};
pub use depth::{AST_CAP, DepthLayer, PRESCAN_CAP};
pub use eval::compile_eval;
pub use function_ctor::compile_function_ctor;
pub use position::{LineMap, Position};
pub use scope::ScopeAnalysis;

/// The ECMA-262 goal symbol of a source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Goal {
    /// Global code (`Script`).
    Script,
    /// Module code (`Module`). Always strict.
    Module,
}

/// Strict or sloppy mode for the top level of a source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Strictness {
    /// Non-strict code. A `"use strict"` directive in the source can still make it strict.
    Sloppy,
    /// Strict code, as if the source started with a `"use strict"` directive.
    Strict,
}

/// How to parse a source text: goal and strictness.
///
/// The fields are private, so a sloppy module cannot exist. Later stages add TS flags (H3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceOpts {
    goal: Goal,
    strictness: Strictness,
}

impl SourceOpts {
    /// Script goal with the given strictness.
    #[must_use]
    pub const fn script(strictness: Strictness) -> Self {
        Self {
            goal: Goal::Script,
            strictness,
        }
    }

    /// Module goal. Module code is always strict.
    #[must_use]
    pub const fn module() -> Self {
        Self {
            goal: Goal::Module,
            strictness: Strictness::Strict,
        }
    }

    /// The goal symbol.
    #[must_use]
    pub const fn goal(self) -> Goal {
        self.goal
    }

    /// The top-level strictness. Always [`Strictness::Strict`] for a module.
    #[must_use]
    pub const fn strictness(self) -> Strictness {
        self.strictness
    }
}

/// A half-open byte range `start..end` into the UTF-8 source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ByteSpan {
    /// First byte.
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
}

impl ByteSpan {
    pub(crate) fn of(span: oxc_span::Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

/// One early error (ECMA-262 "It is a Syntax Error if ...", or a grammar error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    /// Human-readable message. The text comes from oxc and can change on an oxc bump.
    pub message: String,
    /// Where the error is. `None` if oxc gave no location.
    pub span: Option<ByteSpan>,
    /// Line and UTF-16 column of `span.start`. `None` if `span` is `None`.
    pub position: Option<Position>,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some(p) => write!(f, "{}:{}: SyntaxError: {}", p.line, p.column, self.message),
            None => write!(f, "SyntaxError: {}", self.message),
        }
    }
}

/// Every early error found in one source text, in report order. Never empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyErrors {
    errors: Vec<SyntaxError>,
}

impl EarlyErrors {
    /// The first error. A caller that throws one `SyntaxError` throws this one.
    #[must_use]
    pub fn first(&self) -> &SyntaxError {
        &self.errors[0]
    }

    /// All errors.
    #[must_use]
    pub fn as_slice(&self) -> &[SyntaxError] {
        &self.errors
    }

    /// Number of errors (at least 1).
    #[must_use]
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Always `false`; here for API completeness.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Take the errors.
    #[must_use]
    pub fn into_vec(self) -> Vec<SyntaxError> {
        self.errors
    }
}

impl fmt::Display for EarlyErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.first().fmt(f)?;
        if self.errors.len() > 1 {
            write!(f, " (+{} more)", self.errors.len() - 1)?;
        }
        Ok(())
    }
}

impl std::error::Error for EarlyErrors {}

/// Why a [`RangeError`] was raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RangeErrorKind {
    /// Nesting deeper than the cap of a depth-guard layer (H23).
    TooDeep(DepthLayer),
    /// The OS refused the front-end worker stack.
    StackUnavailable,
}

/// A limit was hit while compiling. The engine throws a JS `RangeError`, as V8 does for deep
/// nesting ("Maximum call stack size exceeded").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeError {
    /// Which limit.
    pub kind: RangeErrorKind,
    /// Human-readable message.
    pub message: String,
    /// Where the limit was hit, if known.
    pub span: Option<ByteSpan>,
    /// Line and UTF-16 column of `span.start`.
    pub position: Option<Position>,
}

impl RangeError {
    fn too_deep(layer: DepthLayer, source: &str, at: Option<u32>) -> Self {
        let cap = match layer {
            DepthLayer::PreScan => PRESCAN_CAP,
            DepthLayer::Ast | DepthLayer::Pass => AST_CAP,
        };
        let span = at.map(|start| ByteSpan { start, end: start });
        Self {
            kind: RangeErrorKind::TooDeep(layer),
            message: format!("Maximum call stack size exceeded (nesting depth limit {cap})"),
            span,
            position: span.map(|s| LineMap::new(source.as_bytes()).position(s.start)),
        }
    }
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some(p) => write!(f, "{}:{}: RangeError: {}", p.line, p.column, self.message),
            None => write!(f, "RangeError: {}", self.message),
        }
    }
}

impl std::error::Error for RangeError {}

/// Why a source text did not compile. The engine throws the matching JS error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// Early errors: the engine throws a `SyntaxError`.
    Syntax(EarlyErrors),
    /// A limit: the engine throws a `RangeError`.
    Range(RangeError),
}

impl CompileError {
    /// The early errors, if this is a syntax error.
    #[must_use]
    pub fn as_syntax(&self) -> Option<&EarlyErrors> {
        match self {
            Self::Syntax(errors) => Some(errors),
            Self::Range(_) => None,
        }
    }

    /// The range error, if this is one.
    #[must_use]
    pub fn as_range(&self) -> Option<&RangeError> {
        match self {
            Self::Syntax(_) => None,
            Self::Range(error) => Some(error),
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(errors) => errors.fmt(f),
            Self::Range(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CompileError {}

impl From<EarlyErrors> for CompileError {
    fn from(errors: EarlyErrors) -> Self {
        Self::Syntax(errors)
    }
}

impl From<RangeError> for CompileError {
    fn from(error: RangeError) -> Self {
        Self::Range(error)
    }
}

/// Check `source` for early errors: parse errors, regex literal errors and the static semantics
/// ("early errors") of ECMA-262 and Annex B. `Ok(())` means the source is a valid Script or
/// Module for `opts` and is within the depth caps.
///
/// It runs the depth guard (pre-scan, AST-depth pass) and, on the front-end worker,
/// oxc_parser with `parse_regular_expression`, then (only if the parser found no error)
/// oxc_semantic with syntax checks. Both oxc stages are needed: the parser alone misses about
/// 30% of the test262 parse-phase negatives, and without regex parsing about 8% \[R04\].
///
/// # Errors
///
/// [`CompileError::Syntax`] with every error of the first stage that found one, or
/// [`CompileError::Range`] for nesting above the caps.
pub fn check_early_errors(source: &str, opts: SourceOpts) -> Result<(), CompileError> {
    with_front_end(source, opts, |_| Ok(()))
}

/// Run the whole F1 pipeline and return the scope description of every function.
///
/// # Errors
///
/// As [`check_early_errors`], plus a layer-3 [`RangeError`] from the scope pass.
pub fn analyze(source: &str, opts: SourceOpts) -> Result<ScopeAnalysis, CompileError> {
    with_front_end(source, opts, scope::analyze)
}

/// What a consumer of the front end (the scope pass, later BytecodeGen) gets, on the worker.
pub(crate) struct FrontEnd<'a> {
    pub(crate) source: &'a str,
    pub(crate) opts: SourceOpts,
    pub(crate) program: &'a Program<'a>,
    pub(crate) semantic: &'a Semantic<'a>,
}

/// Layer 0 on the caller's thread, then everything else on the front-end worker.
///
/// `consumer` runs on the worker after oxc_semantic; its result crosses back to the caller.
pub(crate) fn with_front_end<R, F>(
    source: &str,
    opts: SourceOpts,
    consumer: F,
) -> Result<R, CompileError>
where
    R: Send + 'static,
    F: for<'a> FnOnce(&FrontEnd<'a>) -> Result<R, CompileError> + Send + 'static,
{
    let estimate = prescan::estimate(source.as_bytes(), opts.goal == Goal::Module);
    if estimate.depth > PRESCAN_CAP {
        return Err(RangeError::too_deep(DepthLayer::PreScan, source, Some(estimate.at)).into());
    }
    let owned = source.to_owned();
    let job = move || {
        let allocator = Allocator::default();
        let program = parse(&allocator, &owned, opts)?;
        let semantic = semantic(&owned, &program)?;
        consumer(&FrontEnd {
            source: &owned,
            opts,
            program: &program,
            semantic: &semantic,
        })
    };
    match worker::run(stack::WORKER_STACK_BYTES, job) {
        Ok(result) => result,
        Err(spawn) => Err(CompileError::Range(RangeError {
            kind: RangeErrorKind::StackUnavailable,
            message: spawn.message,
            span: None,
            position: None,
        })),
    }
}

/// Parse (with regex validation), make a strict script strict, and run layer 2.
pub(crate) fn parse<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    opts: SourceOpts,
) -> Result<Program<'a>, CompileError> {
    let source_type = match opts.goal {
        Goal::Script => SourceType::script(),
        Goal::Module => SourceType::mjs(),
    };
    let options = ParseOptions {
        parse_regular_expression: true,
        ..ParseOptions::default()
    };
    let mut parsed = Parser::new(allocator, source, source_type)
        .with_options(options)
        .parse();
    if parsed.diagnostics.has_errors() {
        return Err(early_errors(source, parsed.diagnostics.errors()).into());
    }
    if opts.goal == Goal::Script && opts.strictness == Strictness::Strict {
        force_strict(allocator, &mut parsed.program);
    }
    if let Err(too_deep) = depth::check_ast_depth(&parsed.program, AST_CAP) {
        return Err(
            RangeError::too_deep(DepthLayer::Ast, source, Some(too_deep.span.start)).into(),
        );
    }
    Ok(parsed.program)
}

/// oxc_semantic with syntax checks. Call only on a program that passed [`parse`] (layer 2), so
/// the recursive visitors stay within the worker stack. oxc's strict `eval`/`arguments` assign
/// errors on names that are only read go through the PRE-1 post-filter (`strict_filter`).
pub(crate) fn semantic<'a>(
    source: &str,
    program: &'a Program<'a>,
) -> Result<Semantic<'a>, CompileError> {
    let built = SemanticBuilder::new_compiler().build(program);
    if built.diagnostics.has_errors() {
        let kept = strict_filter::kept(program, built.diagnostics.errors());
        if !kept.is_empty() {
            return Err(early_errors(source, kept.into_iter()).into());
        }
    }
    Ok(built.semantic)
}

/// Make a Script strict without changing the source text.
///
/// oxc 0.155 has no strict-script option: only the Module goal is strict. But its parser has no
/// strict state either. Every strict-only early error is in oxc_semantic, which makes the
/// top-level scope strict when `Program::has_use_strict_directive()` holds. So a synthetic
/// directive at index 0 gives the same result as a `"use strict";` source prefix, keeps every
/// byte offset, and works with a hashbang. The test262 runner checks this against the prefix
/// method on the whole suite (`ep9-test262 parse --strict-by opts`).
///
/// The directive has an empty span. oxc_semantic reads string literal text through the span,
/// so it sees an empty raw text and finds no octal escape in it.
fn force_strict<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
    if program.has_use_strict_directive() {
        return;
    }
    let ast = AstBuilder::new(allocator);
    let literal = StringLiteral::new(SPAN, "use strict", None, &ast);
    program
        .directives
        .insert(0, Directive::new(SPAN, literal, "use strict", &ast));
}

fn early_errors<'d>(
    source: &str,
    diagnostics: impl Iterator<Item = &'d OxcDiagnostic>,
) -> EarlyErrors {
    let mut lines: Option<LineMap> = None;
    let errors = diagnostics
        .map(|diagnostic| {
            // Use the primary label if there is one, else the first label.
            let label = diagnostic
                .labels
                .iter()
                .find(|label| label.primary())
                .or_else(|| diagnostic.labels.first());
            let span = label.map(|label| ByteSpan {
                start: label.offset(),
                end: label.offset().saturating_add(label.len()),
            });
            let position = span.map(|span| {
                lines
                    .get_or_insert_with(|| LineMap::new(source.as_bytes()))
                    .position(span.start)
            });
            SyntaxError {
                message: diagnostic.message.to_string(),
                span,
                position,
            }
        })
        .collect::<Vec<_>>();
    assert!(
        !errors.is_empty(),
        "has_errors() was true, so errors() is not empty"
    );
    EarlyErrors { errors }
}

#[cfg(test)]
mod depth_tests;
#[cfg(test)]
mod forms;
#[cfg(test)]
mod probe;
#[cfg(test)]
mod tests;
