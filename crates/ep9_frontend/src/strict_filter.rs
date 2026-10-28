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
//! One exception: oxc's `check_identifier_reference` returns right after the assign diagnostic,
//! so it skips its own class check (§15.7.1: ContainsArguments of a field initializer or a
//! static block is an early error). For an `arguments` read there, the filter does that check:
//!
//! ```text
//! `arguments` read, label S, inside a class field initializer or a static block, with no
//!   ordinary function or method between them (arrows do not count)
//!   -> replace the diagnostic with oxc's own "'arguments' is not allowed in class field
//!      initializer" / "... in static initialization block" at S
//! ```
//!
//! A computed key of a field or method is outside its initializer, so it takes the region
//! around the class (as in oxc and node v24.22.1).
//!
//! The filter never adds a diagnostic that oxc did not report in some form, and runs only when
//! such a diagnostic exists (no cost on the accepted path). The AST walk recurses like oxc_semantic's own walk,
//! so it runs only on a program that passed layer 2 (the caller's order already ensures it).
//! `semantic` in lib.rs and the semantic step of `compile_eval` (eval.rs) use it. Upstream
//! draft: docs/upstream/oxc-strict-eval-default-init.md.

use std::borrow::Cow;

use oxc_ast::ast::{
    AssignmentTargetPropertyIdentifier, Function, IdentifierReference, Program, PropertyDefinition,
    SimpleAssignmentTarget, StaticBlock,
};
use oxc_ast_visit::{Visit, walk};
use oxc_diagnostics::OxcDiagnostic;
use oxc_semantic::ScopeFlags;
use oxc_span::Span;

/// The two messages of oxc_semantic 0.155.0 `diagnostics::unexpected_identifier_assign`.
const MESSAGES: [&str; 2] = [
    "Cannot assign to 'eval' in strict mode",
    "Cannot assign to 'arguments' in strict mode",
];

/// The diagnostics of `errors` that stay: every one except a strict `eval`/`arguments` assign
/// error whose label is a read of the name, and a class `arguments` error in place of such an
/// error inside a field initializer or a static block (see the module doc).
pub(crate) fn kept<'d>(
    program: &Program<'_>,
    errors: impl Iterator<Item = &'d OxcDiagnostic>,
) -> Vec<Cow<'d, OxcDiagnostic>> {
    let errors: Vec<&OxcDiagnostic> = errors.collect();
    if !errors.iter().any(|d| candidate(d).is_some()) {
        return errors.into_iter().map(Cow::Borrowed).collect();
    }
    let mut reads = Reads::default();
    reads.visit_program(program);
    reads.spans.sort_unstable();
    errors
        .into_iter()
        .filter_map(|d| {
            let Some(span) = candidate(d) else {
                return Some(Cow::Borrowed(d));
            };
            match reads.spans.binary_search_by_key(&span, |r| r.0) {
                Err(_) => Some(Cow::Borrowed(d)),
                Ok(i) => reads.spans[i].1.map(|region| {
                    // oxc_semantic 0.155.0 `diagnostics::unexpected_arguments`.
                    Cow::Owned(
                        OxcDiagnostic::error(format!("'arguments' is not allowed in {region}"))
                            .with_label(Span::new(span.0, span.1))
                            .with_help(
                                "Assign the 'arguments' variable to a temporary variable outside",
                            ),
                    )
                }),
            }
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

/// The spans of every `eval`/`arguments` IdentifierReference that is read, not assigned, each
/// with the class region (§15.7.1) an `arguments` read is in, if any.
#[derive(Default)]
struct Reads {
    spans: Vec<((u32, u32), Option<&'static str>)>,
    /// The innermost field initializer or static block around the current node, with no
    /// ordinary function or method in between.
    region: Option<&'static str>,
}

fn is_name(id: &IdentifierReference<'_>) -> bool {
    matches!(id.name.as_str(), "eval" | "arguments")
}

impl<'a> Visit<'a> for Reads {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if is_name(it) {
            let region = if it.name == "arguments" {
                self.region
            } else {
                None
            };
            self.spans.push(((it.span.start, it.span.end), region));
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        // ContainsArguments is false for a function (and a method body); an arrow is not a
        // boundary, so `visit_arrow_function_expression` keeps the walk's default.
        let outer = self.region.take();
        walk::walk_function(self, it, flags);
        self.region = outer;
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        // The key (also a computed one) belongs to the region around the class; only the
        // initializer is the field's own region.
        self.visit_decorators(&it.decorators);
        self.visit_property_key(&it.key);
        if let Some(value) = &it.value {
            let outer = self.region.replace("class field initializer");
            self.visit_expression(value);
            self.region = outer;
        }
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        let outer = self.region.replace("static initialization block");
        walk::walk_static_block(self, it);
        self.region = outer;
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

    /// An `arguments` read in a class field initializer or a static block, inside a
    /// destructuring target (ECMA-262 §15.7.1 ContainsArguments; arrows are not a boundary).
    /// node v24.22.1: "SyntaxError: 'arguments' is not allowed in class field initializer or
    /// static initialization block" for each.
    const CLASS_ARGS: &[&str] = &[
        "class C{x=([v=arguments]=[])}",
        "class C{static{({[arguments]:v}={})}}",
        "class C{static{for([v=arguments] in {});}}",
        "class C{static{for([v=arguments] of []);}}",
        "class C{static x=([v=arguments]=[])}",
        "class C{x=({a:v=arguments}={})}",
        "class C{x=({v=arguments}={})}",
        "class C{x=[v=arguments]=[]}",
        "class C{x=([v=()=>arguments]=[])}",
        "class C{x=([v=()=>()=>arguments]=[])}",
        "class C{x=()=>{[v=arguments]=[]}}",
        "class C{x=([v={[arguments]:1}]=[])}",
        "class C{x=([v={[([w=arguments]=[])](){}}]=[])}",
        "class C{x=class{ y=([v=arguments]=[]) }}",
        "class C{static{ [v=arguments]=[] }}",
        "class C{static{ (()=>{[v=arguments]=[]}) }}",
        "class C{static{ class D{[([v=arguments]=[])]=1} }}",
        "class C{static{ class D{[([v=arguments]=[])](){}} }}",
    ];

    /// The same reads behind an ordinary function or method boundary, or in a class element
    /// key outside a static block: node v24.22.1 accepts each.
    const CLASS_ARGS_OK: &[&str] = &[
        "class C{x=([v=function(){return arguments}]=[])}",
        "class C{x=([v=function(){return ()=>arguments}]=[])}",
        "class C{x=([v={m(){return arguments}}.m]=[])}",
        "class C{[([v=arguments]=[])]=1}",
        "class C{static [([v=arguments]=[])]=1}",
        "class C{[([v=arguments]=[])](){}}",
        "function f(){ class C{[([v=arguments]=[])]=1} }",
        "class C{m(){[v=arguments]=[]}}",
        "class C{x=class{ m(){[v=arguments]=[]} }}",
        "class C{static{ (function(){[v=arguments]=[]}) }}",
        "class C{x=([v=eval]=[])}",
        "class C{static{ [v=eval]=[] }}",
    ];

    fn is_class_args_error(r: Result<(), CompileError>) -> bool {
        match r {
            Err(CompileError::Syntax(e)) => e.errors.iter().any(|e| {
                e.message == "'arguments' is not allowed in class field initializer"
                    || e.message == "'arguments' is not allowed in static initialization block"
            }),
            _ => false,
        }
    }

    fn class_args_codegen_error<T>(r: &Result<T, crate::CodegenError>) -> bool {
        match r {
            Err(crate::CodegenError::Compile(e)) => is_class_args_error(Err(e.clone())),
            _ => false,
        }
    }

    #[test]
    fn class_arguments_reads_are_errors_in_scripts() {
        for src in CLASS_ARGS {
            let src = format!("var v, w; {src}");
            for opts in [
                SourceOpts::script(Strictness::Strict),
                SourceOpts::script(Strictness::Sloppy),
                SourceOpts::module(),
            ] {
                let r = check_early_errors(&src, opts);
                assert!(is_class_args_error(r.clone()), "{src} {opts:?}: {r:?}");
                let c = crate::compile(&src, opts, crate::CompileOptions::default());
                assert!(
                    class_args_codegen_error(&c),
                    "{src} {opts:?}: {:?}",
                    c.err()
                );
            }
        }
        for src in CLASS_ARGS_OK {
            let src = format!("var v, w; {src}");
            for opts in [
                SourceOpts::script(Strictness::Strict),
                SourceOpts::script(Strictness::Sloppy),
            ] {
                assert_eq!(check_early_errors(&src, opts), Ok(()), "{src} {opts:?}");
            }
        }
    }

    #[test]
    fn class_arguments_reads_are_errors_in_eval_code() {
        use ep9_bytecode::EvalContext;
        let options = crate::CompileOptions::default();
        for strict in [true, false] {
            let ctx = EvalContext {
                strict,
                ..EvalContext::default()
            };
            for src in CLASS_ARGS {
                let src = format!("var v, w; {src}");
                let r = crate::compile_eval(&src, &ctx, options);
                assert!(
                    class_args_codegen_error(&r),
                    "{src} strict={strict}: {:?}",
                    r.err()
                );
            }
            for src in CLASS_ARGS_OK {
                let src = format!("var v, w; {src}");
                let r = crate::compile_eval(&src, &ctx, options);
                assert!(r.is_ok(), "{src} strict={strict}: {:?}", r.err());
            }
        }
    }

    #[test]
    fn class_arguments_reads_are_errors_in_function_ctor_bodies() {
        use ep9_bytecode::FunctionCtorKind;
        let options = crate::CompileOptions::default();
        for src in CLASS_ARGS {
            let body = format!("var v, w; {src}");
            let r = crate::compile_function_ctor(FunctionCtorKind::Normal, "", &body, options);
            assert!(class_args_codegen_error(&r), "{body}: {:?}", r.err());
        }
        for src in CLASS_ARGS_OK {
            let body = format!("var v, w; {src}");
            let r = crate::compile_function_ctor(FunctionCtorKind::Normal, "", &body, options);
            assert!(r.is_ok(), "{body}: {:?}", r.err());
        }
    }
}
