//! The front end and the only crate that imports oxc: parse, semantic, depth guard, scope pass,
//! `BytecodeGen` with TS lowering, hint join from `.ep9h`, lone-surrogate shim and L1 cache.
//!
//! v0 (M0 week 1) has one entry, [`check_early_errors`]:
//!
//! ```text
//! source --> oxc_parser (+ regex validation) --errors?--> Err(EarlyErrors)
//!                 | no errors
//!                 v
//!            strict script? insert a "use strict" directive into the AST
//!                 v
//!            oxc_semantic syntax checks --------errors?--> Err(EarlyErrors)
//!                 v
//!               Ok(())
//! ```
//!
//! - No oxc type leaves this crate (H1). Errors are plain [`SyntaxError`] values.
//! - The source type is always set explicitly: Script or Module goal, JavaScript, no JSX.
//! - Stack: oxc recurses once per nesting level and has no depth limit yet (01-front-end §6).
//!   Until the depth guard lands (M0 week 3), run this on a thread with a large stack.

use std::fmt;

use oxc_allocator::Allocator;
use oxc_ast::ast::{Directive, Program, StringLiteral};
use oxc_ast::builder::AstBuilder;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::SemanticBuilder;
use oxc_span::{SPAN, SourceType};

mod position;

use position::LineIndex;
pub use position::Position;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ByteSpan {
    /// First byte.
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
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

/// Check `source` for early errors: parse errors, regex literal errors and the static semantics
/// ("early errors") of ECMA-262 and Annex B. `Ok(())` means the source is a valid
/// Script or Module for `opts`.
///
/// It runs oxc_parser with `parse_regular_expression`, then (only if the parser found no error)
/// oxc_semantic with syntax checks. Both are needed: the parser alone misses about 30% of the
/// test262 parse-phase negatives, and without regex parsing about 8% [R04].
///
/// # Errors
///
/// Returns every error of the first stage that found one.
pub fn check_early_errors(source: &str, opts: SourceOpts) -> Result<(), EarlyErrors> {
    let allocator = Allocator::default();
    let source_type = match opts.goal {
        Goal::Script => SourceType::script(),
        Goal::Module => SourceType::mjs(),
    };
    let options = ParseOptions {
        parse_regular_expression: true,
        ..ParseOptions::default()
    };
    let mut parsed = Parser::new(&allocator, source, source_type)
        .with_options(options)
        .parse();
    if parsed.diagnostics.has_errors() {
        return Err(early_errors(source, parsed.diagnostics.errors()));
    }

    if opts.goal == Goal::Script && opts.strictness == Strictness::Strict {
        force_strict(&allocator, &mut parsed.program);
    }

    let semantic = SemanticBuilder::new_compiler().build(&parsed.program);
    if semantic.diagnostics.has_errors() {
        return Err(early_errors(source, semantic.diagnostics.errors()));
    }
    Ok(())
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
    let mut lines: Option<LineIndex<'_>> = None;
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
                    .get_or_insert_with(|| LineIndex::new(source))
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
mod tests;
