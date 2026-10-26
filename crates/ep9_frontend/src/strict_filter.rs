//! The strict `eval`/`arguments` post-filter for oxc_semantic diagnostics (PRE-1).
//!
//! oxc_semantic 0.155.0 (`src/checker/javascript.rs`, `check_identifier_reference`) reports
//! "Cannot assign to 'eval' in strict mode" (or `'arguments'`) for an `eval`/`arguments`
//! IdentifierReference in strict code when any ancestor is an `ArrayAssignmentTarget`,
//! `ObjectAssignmentTarget`, `AssignmentTargetPropertyIdentifier` or `UpdateExpression`. The
//! walk stops only at a member expression, so a reference that is only read inside a
//! destructuring target also gets the error:
//!
//! ```text
//! "use strict"; [v = eval("1")] = [];          oxc: SyntaxError   node v24.22.1: v = 1
//! "use strict"; ({ [eval("'k'")]: v } = o);    oxc: SyntaxError   node: ok
//! "use strict"; [v = () => arguments] = [];    oxc: SyntaxError   node: ok
//! ```
//!
//! The spec makes `eval` and `arguments` an early error only where the IdentifierReference is
//! itself an assignment target (AssignmentTargetType, ECMA-262 §13.1.1) or a binding
//! (§13.1.1 BindingIdentifier). Reading them is always allowed. So, after oxc_semantic:
//!
//! ```text
//! diagnostic "Cannot assign to '<eval|arguments>' in strict mode" with label span S
//!   S = the span of an IdentifierReference that is NOT an assignment target -> drop
//!   anything else (a target, a BindingIdentifier, no label)                 -> keep
//! assignment target = SimpleAssignmentTarget::AssignmentTargetIdentifier (the left side of
//!   `=`/`op=`, an element or a rest of an array pattern, the value of `{a: x}` or `{...x}`, a
//!   for-in/of head, the operand of `++`/`--`) or the binding of a shorthand `{x}` / `{x = 1}`
//! ```
//!
//! The filter only removes diagnostics, never adds one, and runs only when such a diagnostic
//! exists (no cost on the accepted path). The AST walk recurses like oxc_semantic's own walk,
//! so it runs only on a program that passed layer 2 (the caller's order already ensures it).
//! `semantic` in lib.rs and the semantic step of `compile_eval` (eval.rs) use it. Upstream
//! draft: docs/upstream/oxc-strict-eval-default-init.md.

use oxc_ast::ast::{
    AssignmentTargetPropertyIdentifier, IdentifierReference, Program, SimpleAssignmentTarget,
};
use oxc_ast_visit::{Visit, walk};
use oxc_diagnostics::OxcDiagnostic;

/// The two messages of oxc_semantic 0.155.0 `diagnostics::unexpected_identifier_assign`.
const MESSAGES: [&str; 2] = [
    "Cannot assign to 'eval' in strict mode",
    "Cannot assign to 'arguments' in strict mode",
];

/// The diagnostics of `errors` that stay: every one except a strict `eval`/`arguments` assign
/// error whose label is a read of the name (see the module doc).
pub(crate) fn kept<'d>(
    program: &Program<'_>,
    errors: impl Iterator<Item = &'d OxcDiagnostic>,
) -> Vec<&'d OxcDiagnostic> {
    let errors: Vec<&OxcDiagnostic> = errors.collect();
    if !errors.iter().any(|d| candidate(d).is_some()) {
        return errors;
    }
    let mut reads = Reads::default();
    reads.visit_program(program);
    reads.spans.sort_unstable();
    errors
        .into_iter()
        .filter(|d| match candidate(d) {
            Some(span) => reads.spans.binary_search(&span).is_err(),
            None => true,
        })
        .collect()
}

/// The label span `(start, end)` of a strict `eval`/`arguments` assign diagnostic, or `None`
/// for any other diagnostic.
fn candidate(d: &OxcDiagnostic) -> Option<(u32, u32)> {
    if !MESSAGES.contains(&&*d.message) {
        return None;
    }
    let label = d
        .labels
        .iter()
        .find(|l| l.primary())
        .or_else(|| d.labels.first())?;
    let start = label.offset();
    Some((start, start.checked_add(label.len())?))
}

/// The spans of every `eval`/`arguments` IdentifierReference that is read, not assigned.
#[derive(Default)]
struct Reads {
    spans: Vec<(u32, u32)>,
}

fn is_name(id: &IdentifierReference<'_>) -> bool {
    matches!(id.name.as_str(), "eval" | "arguments")
}

impl<'a> Visit<'a> for Reads {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if is_name(it) {
            self.spans.push((it.span.start, it.span.end));
        }
    }

    fn visit_simple_assignment_target(&mut self, it: &SimpleAssignmentTarget<'a>) {
        // A target name is not a read: do not visit it. Member targets (`a[eval(0)]`) walk
        // on, their names are reads.
        if !matches!(it, SimpleAssignmentTarget::AssignmentTargetIdentifier(_)) {
            walk::walk_simple_assignment_target(self, it);
        }
    }

    fn visit_assignment_target_property_identifier(
        &mut self,
        it: &AssignmentTargetPropertyIdentifier<'a>,
    ) {
        // `{x}` and `{x = init}`: `x` is the target; the initializer is read.
        if let Some(init) = &it.init {
            self.visit_expression(init);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{CompileError, SourceOpts, Strictness, check_early_errors};

    fn strict(src: &str) -> Result<(), CompileError> {
        check_early_errors(src, SourceOpts::script(Strictness::Strict))
    }

    fn assign_error(src: &str) -> String {
        match strict(src) {
            Err(CompileError::Syntax(e)) => e.errors[0].message.clone(),
            other => panic!("{src}: expected a SyntaxError, got {other:?}"),
        }
    }

    /// The 6 PRE-1 forms (gc-front-fuzz §1.4) and more reads: node v24.22.1 accepts them all
    /// (the run values are in the FE-FUZZ report).
    const READS: &[&str] = &[
        r#"var v; [v = eval("1")] = [];"#,
        "var v; [v = eval] = [];",
        r#"var v; ({ v = eval("2") } = {});"#,
        r#"var v; ({ a: v = eval("3") } = {});"#,
        r#"var v; for ([v = eval("5")] of [[]]);"#,
        r#"var a = []; [a[eval("0")]] = [4];"#,
        "var v; [v = function(){ return eval }] = [];",
        "var v; [v = () => arguments] = [];",
        "function f(){ var v; [v = arguments] = []; return v }",
        r#"var v; ({ [eval("'k'")]: v } = { k: 7 });"#,
        r#"var o = {}; [o[eval("'p'")] = 8] = [];"#,
        r#"var v; [{ v = eval("9") }] = [{}];"#,
        "var v; ({ v = eval } = {});",
        "var v; [v = class { m(){ return eval } }] = [];",
        "var v; ({ v = arguments } = {});",
        "var v; [...[v = eval]] = [];",
    ];

    /// Real targets and bindings: node v24.22.1 throws a SyntaxError for each.
    const TARGETS: &[&str] = &[
        "[eval] = []",
        "({eval} = {})",
        "eval++",
        "--eval",
        "eval = 1",
        "eval += 1",
        "eval ??= 1",
        "(eval) = 1",
        "[(eval)] = []",
        "({a: eval} = {})",
        "({eval = 1} = {})",
        "[...eval] = []",
        "({...eval} = {})",
        "[arguments] = []",
        "var x = [arguments] = []",
        "var v; [v = (eval = 1)] = []",
        "var v; [v = eval++] = []",
        "var v; [v = eval] = []; arguments = 1",
        "function g(eval){}",
        "let eval = 1",
        "var v; [eval = v] = []",
    ];

    #[test]
    fn reads_are_accepted() {
        for src in READS {
            assert_eq!(strict(src), Ok(()), "{src}");
        }
    }

    #[test]
    fn targets_keep_their_error() {
        for src in TARGETS {
            let message = assign_error(src);
            assert!(
                message.starts_with("Cannot assign to '"),
                "{src}: {message}"
            );
        }
    }

    #[test]
    fn sloppy_code_is_unchanged() {
        for src in READS
            .iter()
            .chain(&["[eval] = []", "eval++", "({eval} = {})"])
        {
            assert_eq!(
                check_early_errors(src, SourceOpts::script(Strictness::Sloppy)),
                Ok(()),
                "{src}"
            );
        }
    }

    #[test]
    fn other_errors_stay() {
        // A read is dropped, the real error next to it stays.
        let src = "var v; [v = eval] = []; let let = 1;";
        assert!(strict(src).is_err());
        // A module is strict too.
        assert_eq!(
            check_early_errors("var v; [v = eval] = [];", SourceOpts::module()),
            Ok(())
        );
        assert!(check_early_errors("[eval] = []", SourceOpts::module()).is_err());
    }

    #[test]
    fn eval_code_uses_the_filter() {
        use ep9_bytecode::EvalContext;
        let ctx = EvalContext {
            strict: true,
            ..EvalContext::default()
        };
        let options = crate::CompileOptions::default();
        for src in READS {
            let r = crate::compile_eval(src, &ctx, options);
            assert!(r.is_ok(), "{src}: {:?}", r.err());
        }
        for src in TARGETS {
            let r = crate::compile_eval(src, &ctx, options);
            assert!(r.is_err(), "{src}");
        }
    }
}
