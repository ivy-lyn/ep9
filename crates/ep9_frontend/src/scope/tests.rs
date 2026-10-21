//! Scope pass tests: many small programs, each checked against ECMA-262 behavior (and against
//! node 24 where the spec leaves a choice). The last tests compare with oxc's own resolution.

use oxc_semantic::ReferenceId;

use super::*;
use crate::{SourceOpts, Strictness, with_front_end};

fn run(src: &str, opts: SourceOpts) -> ScopeAnalysis {
    crate::analyze(src, opts).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

fn script(src: &str) -> ScopeAnalysis {
    run(src, SourceOpts::script(Strictness::Sloppy))
}

fn strict(src: &str) -> ScopeAnalysis {
    run(src, SourceOpts::script(Strictness::Strict))
}

fn module(src: &str) -> ScopeAnalysis {
    run(src, SourceOpts::module())
}

/// Test lookups by name text.
impl ScopeAnalysis {
    fn named(&self, name: &str) -> Vec<BindingId> {
        (0..self.bindings.len())
            .map(BindingId::of)
            .filter(|&b| self.name(self.binding(b).name) == name)
            .collect()
    }

    /// The only binding called `name`.
    fn the(&self, name: &str) -> &Binding {
        let all = self.named(name);
        assert_eq!(all.len(), 1, "bindings named {name}:\n{self}");
        self.binding(all[0])
    }

    fn id_of(&self, name: &str) -> BindingId {
        let all = self.named(name);
        assert_eq!(all.len(), 1, "bindings named {name}:\n{self}");
        all[0]
    }

    fn loc(&self, name: &str) -> BindingLocation {
        self.the(name).location
    }

    /// References to `name`, in walk order.
    fn refs(&self, name: &str) -> Vec<&Reference> {
        self.references
            .iter()
            .filter(|r| self.name(r.name) == name)
            .collect()
    }

    /// The only reference to `name`.
    fn res(&self, name: &str) -> Resolution {
        let all = self.refs(name);
        assert_eq!(all.len(), 1, "references to {name}:\n{self}");
        all[0].resolution
    }

    /// The binding the `nth` reference to `name` resolves to (statically).
    fn target(&self, name: &str, nth: usize) -> BindingId {
        match self.refs(name)[nth].resolution {
            Resolution::Binding { binding, .. } => binding,
            other => panic!("reference {nth} to {name}: {other:?}\n{self}"),
        }
    }

    fn hole_checks(&self, name: &str) -> Vec<bool> {
        self.refs(name)
            .iter()
            .map(|r| match r.resolution {
                Resolution::Binding { hole_check, .. } => hole_check,
                other => panic!("{name}: {other:?}"),
            })
            .collect()
    }

    fn func(&self, i: u32) -> &FunctionInfo {
        self.function(FunctionId(i))
    }
}

fn ctx(a: &ScopeAnalysis, name: &str) -> (ScopeId, ContextReason) {
    let b = a.the(name);
    match b.location {
        BindingLocation::Context { scope, .. } => (scope, b.context_reason.expect("a reason")),
        other => panic!("{name} is not in a context: {other:?}\n{a}"),
    }
}

fn is_register(loc: BindingLocation) -> bool {
    matches!(loc, BindingLocation::Register(_))
}

// Register locals and parameters.

#[test]
fn uncaptured_locals_are_registers() {
    let a =
        script("function f(a, b) { var x = 1; let y = 2; const z = 3; return a + b + x + y + z; }");
    assert_eq!(a.loc("a"), BindingLocation::Parameter(0));
    assert_eq!(a.loc("b"), BindingLocation::Parameter(1));
    assert!(is_register(a.loc("x")) && is_register(a.loc("y")) && is_register(a.loc("z")));
    let f = a.func(1);
    assert_eq!(f.register_count, 3);
    assert!(a.scopes().iter().all(|s| !s.needs_context), "{a}");
    assert_eq!(a.the("f").location, BindingLocation::Global);
}

#[test]
fn registers_are_numbered_in_declaration_order_per_function() {
    let a = script("function f() { var p; { let q; } { let r; } function g() { var s; } }");
    assert_eq!(a.loc("p"), BindingLocation::Register(0));
    assert_eq!(a.loc("q"), BindingLocation::Register(2));
    assert_eq!(a.loc("r"), BindingLocation::Register(3));
    // g is the function declaration binding in f: register 1 (declared before the blocks).
    assert_eq!(a.loc("g"), BindingLocation::Register(1));
    assert_eq!(a.loc("s"), BindingLocation::Register(0));
}

#[test]
fn sloppy_duplicate_parameters_bind_the_last_slot() {
    let a = script("function f(a, a) { return a; }");
    assert_eq!(a.loc("a"), BindingLocation::Parameter(1));
    let f = a.func(1);
    assert_eq!(f.params[0].binding, None);
    assert_eq!(f.params[1].binding, Some(a.id_of("a")));
}

#[test]
fn destructured_and_rest_parameters_are_locals() {
    let a = strict("function f({ p }, [q], ...r) { return p + q + r; }");
    assert!(is_register(a.loc("p")) && is_register(a.loc("q")) && is_register(a.loc("r")));
    let f = a.func(1);
    assert!(!f.simple_params);
    assert_eq!(f.params.len(), 3);
    assert!(f.params[2].rest);
}

// Closures and context slots.

#[test]
fn captured_bindings_go_to_context_slots() {
    let a = script("function f() { var x; let y; return () => x; }");
    let (scope, reason) = ctx(&a, "x");
    assert_eq!(scope, a.func(1).scope);
    assert_eq!(reason, ContextReason::Captured);
    assert!(is_register(a.loc("y")));
    assert_eq!(
        a.res("x"),
        Resolution::Binding {
            binding: a.id_of("x"),
            hops: 0,
            hole_check: false
        }
    );
    assert!(a.scope(scope).needs_context);
    assert_eq!(a.scope(scope).context_slots, vec![a.id_of("x")]);
}

#[test]
fn hops_count_the_contexts_in_between() {
    let a = script(
        "function f() { let a = 1; { let b = 2; return () => { let c = 3; return () => a + b + c; }; } }",
    );
    let hops = |name: &str| match a.res(name) {
        Resolution::Binding { hops, .. } => hops,
        other => panic!("{other:?}"),
    };
    // inner arrow -> (c's arrow scope: context) -> (block: context for b) -> f scope (a)
    assert_eq!(hops("c"), 0);
    assert_eq!(hops("b"), 1);
    assert_eq!(hops("a"), 2);
}

#[test]
fn shadowing_resolves_to_the_nearest_declaration() {
    let a = script("var x = 0; function f(x) { { let x = 2; x; } return x; } x;");
    let all = a.named("x");
    assert_eq!(all.len(), 3);
    let refs = a.refs("x");
    // var x = 0 (VarInit), x in block, x in return, top-level x.
    assert_eq!(refs.len(), 4);
    let target = |i: usize| match refs[i].resolution {
        Resolution::Binding { binding, .. } => binding,
        other => panic!("{other:?}"),
    };
    assert_eq!(a.binding(target(1)).kind, BindingKind::Let);
    assert_eq!(a.binding(target(2)).kind, BindingKind::Parameter);
    assert_eq!(a.binding(target(3)).kind, BindingKind::Var);
}

// Global and module code.

#[test]
fn script_top_level_declarations_are_global() {
    let a = script("var v; function f() {} let l; const c = 1; class K {} undeclared;");
    assert_eq!(a.loc("v"), BindingLocation::Global);
    assert_eq!(a.loc("f"), BindingLocation::Global);
    assert_eq!(a.loc("l"), BindingLocation::GlobalLexical);
    assert_eq!(a.loc("c"), BindingLocation::GlobalLexical);
    let k = a.named("K");
    assert_eq!(k.len(), 2, "outer and inner class name");
    assert_eq!(a.binding(k[0]).location, BindingLocation::GlobalLexical);
    assert_eq!(a.binding(k[1]).kind, BindingKind::ClassInner);
    assert_eq!(a.res("undeclared"), Resolution::Global);
}

#[test]
fn module_bindings() {
    let a = module(
        "import d, { n as m } from 'x'; import * as ns from 'y';
         export let e = 1; let c = 2; let hidden = 3; let local = 4;
         export { c as renamed };
         export function g() { return d + m + ns + e + c + hidden; }
         local;",
    );
    for name in ["d", "m", "ns", "e", "c", "g"] {
        assert_eq!(a.loc(name), BindingLocation::Module, "{name}\n{a}");
    }
    assert_eq!(ctx(&a, "hidden").1, ContextReason::Captured);
    assert!(is_register(a.loc("local")));
    // Named imports may be uninitialized (cycles): always checked. Namespaces never.
    assert_eq!(a.hole_checks("d"), vec![true]);
    assert_eq!(a.hole_checks("ns"), vec![false]);
    // `this` is never used: no binding.
    assert!(a.named("this").is_empty());
    let a = module("() => this;");
    assert_eq!(ctx(&a, "this").0, ScopeId(0));
}

#[test]
fn module_default_exports() {
    let a = module("export default function () {} ");
    assert_eq!(a.the("*default*").kind, BindingKind::Function);
    assert!(a.the("*default*").exported);
    let a = module("export default class {} ");
    assert_eq!(a.the("*default*").kind, BindingKind::Class);
    let a = module("export default 1 + 2;");
    assert_eq!(a.the("*default*").kind, BindingKind::Let);
    assert_eq!(a.loc("*default*"), BindingLocation::Module);
    let a = module("export default function named() {} named;");
    assert_eq!(a.loc("named"), BindingLocation::Module);
}

// Direct eval, with: dynamic lookups.

#[test]
fn sloppy_direct_eval_forces_contexts_and_dynamic_lookups() {
    let a = script(
        "var y; function f(a) { var x; eval('1'); function g() { return y; } return x + z; }",
    );
    let f = a.func(1);
    assert!(f.calls_eval && f.calls_sloppy_eval);
    assert!(a.scope(f.scope).sloppy_eval && a.scope(f.scope).needs_context);
    for name in ["a", "x", "g", "arguments", "this"] {
        assert_eq!(ctx(&a, name).1, ContextReason::Eval, "{name}\n{a}");
    }
    assert!(f.mapped_arguments);
    // x is in the eval's var scope itself: eval can only redeclare it, so the lookup is static.
    assert!(matches!(a.res("x"), Resolution::Binding { .. }));
    // z and the y in g pass the eval's var scope: eval may add them at run time.
    assert_eq!(a.res("z"), Resolution::Dynamic { fallback: None });
    let ys = a.refs("y");
    assert_eq!(
        ys.last().unwrap().resolution,
        Resolution::Dynamic {
            fallback: Some(a.id_of("y"))
        }
    );
}

/// C6 (M2 P0): binding 0 and context slot 0 of a sloppy-eval function var scope are
/// `.eval_ext`; the script scope gets none.
#[test]
fn sloppy_eval_function_var_scopes_get_eval_ext_at_slot_0() {
    let a = script("eval('var t'); function f(a) { var x; eval('1'); }");
    let ext = a.id_of(ep9_bytecode::EVAL_EXT_BINDING);
    let f = a.func(1);
    let fs = a.scope(f.scope);
    assert_eq!(fs.bindings[0], ext, "{a}");
    assert_eq!(fs.context_slots[0], ext, "{a}");
    let b = a.binding(ext);
    assert_eq!(b.kind, BindingKind::EvalExt);
    assert!(b.kind.is_implicit() && !b.tdz);
    assert_eq!(
        b.location,
        BindingLocation::Context {
            scope: f.scope,
            slot: 0
        }
    );
    // Script scope 0 is a sloppy-eval scope too, but its eval vars are global properties.
    assert!(a.scope(ScopeId(0)).sloppy_eval);
    assert!(!a.scope(ScopeId(0)).bindings.contains(&ext));
    // Strict code never gets one.
    let s = strict("function f() { eval('var x'); }");
    assert!(s.named(ep9_bytecode::EVAL_EXT_BINDING).is_empty());
}

#[test]
fn strict_direct_eval_reads_but_never_adds_bindings() {
    let a = strict("function f() { var x; eval('x'); return x + z; }");
    let f = a.func(1);
    assert!(f.calls_eval && !f.calls_sloppy_eval);
    assert_eq!(ctx(&a, "x").1, ContextReason::Eval);
    assert_eq!(a.res("z"), Resolution::Global);
    assert!(f.arguments.is_some() && !f.mapped_arguments);
}

#[test]
fn eval_in_an_arrow_uses_the_enclosing_this_and_arguments() {
    let a = script("function f() { return () => eval('this'); }");
    let f = a.func(1);
    assert_eq!(ctx(&a, "this").1, ContextReason::Captured);
    assert!(f.arguments.is_some());
    assert!(f.new_target.is_some());
    assert!(a.func(2).calls_eval);
}

#[test]
fn which_calls_are_direct_eval() {
    for (src, direct) in [
        ("eval(1)", true),
        ("(eval)(1)", true),
        ("((eval))(1)", true),
        ("eval?.(1)", false),
        ("(0, eval)(1)", false),
        ("window.eval(1)", false),
        ("new eval(1)", false),
    ] {
        let a = script(&format!("function f() {{ {src}; }}"));
        assert_eq!(a.func(1).calls_eval, direct, "{src}");
    }
}

#[test]
fn eval_in_parameters_adds_vars_outside_the_parameters() {
    // Sloppy, split: eval vars go to the function environment outside the parameter scope.
    let a = script("function f(a = eval('var z'), b = () => z + a) { var a2; return z; }");
    let f = a.func(1);
    assert!(f.has_parameter_expressions);
    assert!(a.scope(f.scope).sloppy_eval);
    assert!(!a.scope(f.body_scope).sloppy_eval);
    for r in a.refs("z") {
        assert_eq!(r.resolution, Resolution::Dynamic { fallback: None });
    }
    // `a` is found in the parameter scope before passing it.
    assert!(matches!(
        a.refs("a")[0].resolution,
        Resolution::Binding { .. }
    ));
}

#[test]
fn with_makes_lookups_dynamic() {
    let a = script("function f(o) { var x; with (o) { x; y; } return x; }");
    let xs = a.refs("x");
    assert_eq!(
        xs[0].resolution,
        Resolution::Dynamic {
            fallback: Some(a.id_of("x"))
        }
    );
    assert!(matches!(xs[1].resolution, Resolution::Binding { .. }));
    assert_eq!(ctx(&a, "x").1, ContextReason::DynamicLookup);
    assert_eq!(a.res("y"), Resolution::Dynamic { fallback: None });
    // The with object is evaluated outside the with scope.
    assert!(matches!(a.res("o"), Resolution::Binding { .. }));
    let with = a
        .scopes()
        .iter()
        .find(|s| s.kind == ScopeKind::With)
        .unwrap();
    assert!(with.needs_context);
}

#[test]
fn with_does_not_affect_this() {
    // Private names cannot be inside a `with`: class code is strict.
    let a = script("function f(o) { with (o) { this; () => this; } }");
    for r in a.refs("this") {
        assert!(matches!(r.resolution, Resolution::Binding { .. }), "{a}");
    }
}

// TDZ.

#[test]
fn tdz_checks_follow_the_v8_position_rule() {
    let a = script("function f() { x; let x = 1; x; return () => x; }");
    assert_eq!(a.hole_checks("x"), vec![true, false, true]);
    let a = script("function f() { let y = y; }");
    assert_eq!(a.hole_checks("y"), vec![true]);
    let a = script("function f() { const c = 1, d = c; }");
    assert_eq!(a.hole_checks("c"), vec![false]);
    let a = script("function f() { var v; v; }");
    assert_eq!(a.hole_checks("v"), vec![false]);
}

#[test]
fn switch_cases_always_check() {
    let a = script("function f(k) { switch (k) { case 0: let s = 1; case 1: s; } }");
    assert_eq!(a.hole_checks("s"), vec![true]);
}

#[test]
fn class_names_are_in_tdz_until_the_class_is_defined() {
    let a = script("function f() { class C extends (0, C) {} C; }");
    let refs = a.refs("C");
    // Heritage: the inner binding, before the class is defined.
    assert_eq!(a.binding(a.target("C", 0)).kind, BindingKind::ClassInner);
    assert_eq!(
        refs[0].resolution,
        Resolution::Binding {
            binding: a.target("C", 0),
            hops: 0,
            hole_check: true
        }
    );
    // After the declaration: the outer binding, initialized.
    assert_eq!(a.binding(a.target("C", 1)).kind, BindingKind::Class);
    assert_eq!(a.hole_checks("C"), vec![true, false]);
}

#[test]
fn for_of_head_is_in_tdz() {
    let a = script("function f() { for (let x of x) { x; } }");
    assert_eq!(a.hole_checks("x"), vec![true, false]);
}

#[test]
fn non_simple_parameters_have_a_tdz() {
    let a = script("function f(a = b, b) { return a + b; }");
    assert_eq!(a.hole_checks("b"), vec![true, false]);
    let a = script("function f(a, b = a) {}");
    assert_eq!(a.hole_checks("a"), vec![false]);
    let a = script("function f(a, b) { return a + b; }");
    assert!(!a.the("a").tdz);
}

#[test]
fn catch_patterns_have_a_tdz() {
    // In the pattern: checked. In the body: not. In a closure: checked.
    let a = script("function f() { try {} catch ({ a = a, b = () => a }) { a; () => a; } }");
    assert!(a.the("a").tdz);
    assert_eq!(a.hole_checks("a"), vec![true, true, false, true]);
    let a = script("function f() { try {} catch ([a = b, b]) { b; } }");
    assert_eq!(a.hole_checks("b"), vec![true, false]);
    // A plain catch parameter is initialized at once.
    let a = script("function f() { try {} catch (e) { e; () => e; } }");
    assert!(!a.the("e").tdz);
    assert_eq!(a.hole_checks("e"), vec![false, false]);
}

// Hoisting.

#[test]
fn var_and_function_declarations_hoist() {
    let a = script("function f() { g(); v; { var v = 1; } function g() {} }");
    let f = a.func(1);
    let v = a.the("v");
    assert_eq!(v.scope, f.scope);
    assert_eq!(a.hole_checks("v"), vec![false, false]);
    let g = a.id_of("g");
    assert_eq!(a.scope(f.scope).hoisted_functions, vec![(g, FunctionId(2))]);
    assert_eq!(a.binding(g).kind, BindingKind::Function);
}

#[test]
fn a_function_declaration_shares_a_var_or_parameter_binding() {
    let a = script("function f(p) { var q; function p() {} function q() {} }");
    assert_eq!(a.named("p").len(), 1);
    assert_eq!(a.named("q").len(), 1);
    assert_eq!(a.the("q").kind, BindingKind::Function);
    assert_eq!(a.the("p").kind, BindingKind::Parameter);
    assert_eq!(a.scope(a.func(1).scope).hoisted_functions.len(), 2);
}

#[test]
fn block_functions_are_lexical_and_hoisted_in_their_block() {
    let a = strict("function f() { { g(); function g() {} } }");
    let g = a.the("g");
    assert_eq!(g.kind, BindingKind::BlockFunction);
    assert_eq!(a.scope(g.scope).kind, ScopeKind::Block);
    assert_eq!(a.scope(g.scope).hoisted_functions.len(), 1);
    assert_eq!(g.annex_b_var, None, "strict code has no Annex B");
}

// Annex B.3.2 (test262 annexB/language/function-code).

fn annex_b(src: &str) -> Vec<bool> {
    let a = script(src);
    let mut out = Vec::new();
    for b in a.bindings() {
        if b.kind == BindingKind::BlockFunction {
            out.push(b.annex_b_var.is_some());
        }
    }
    out
}

#[test]
fn annex_b_block_functions_get_a_var_binding() {
    let a = script("function f() { { function g() {} } return g; }");
    let block = a.named("g");
    assert_eq!(block.len(), 2, "{a}");
    let (lexical, var) = (block[0], block[1]);
    assert_eq!(a.binding(lexical).annex_b_var, Some(var));
    assert_eq!(a.binding(var).kind, BindingKind::Var);
    assert_eq!(a.binding(var).scope, a.func(1).scope);
    // `return g` outside the block reads the var binding.
    assert_eq!(a.target("g", 0), var);
    // At script level the var binding is global.
    let a = script("{ function h() {} }");
    let var = a.binding(a.named("h")[1]);
    assert_eq!(var.location, BindingLocation::Global);
}

#[test]
fn annex_b_skips_on_conflicts() {
    // Nested blocks: the inner function would conflict with the outer block function.
    assert_eq!(
        annex_b("function g() { { function f() { return 1; } { function f() { return 2; } } } }"),
        vec![true, false]
    );
    assert_eq!(
        annex_b("function g() { let f; { function f() {} } }"),
        vec![false]
    );
    assert_eq!(
        annex_b("function g(f) { { function f() {} } }"),
        vec![false]
    );
    assert_eq!(
        annex_b("function g() { { let f; { function f() {} } } }"),
        vec![false]
    );
    assert_eq!(
        annex_b("function g() { for (let f of []) { function f() {} } }"),
        vec![false]
    );
    // B.3.4: a simple catch parameter does not block it, a pattern does.
    assert_eq!(
        annex_b("function g() { try {} catch (f) { { function f() {} } } }"),
        vec![true]
    );
    assert_eq!(
        annex_b("function g() { try {} catch ({ f }) { { function f() {} } } }"),
        vec![false]
    );
    // Two declarations in one block: hoisted, as V8 and SpiderMonkey do.
    assert_eq!(annex_b("{ function a() {} function a() {} }"), vec![true]);
    // arguments with a needed arguments object: skipped (spec and the main test262 test
    // block-decl-func-skip-arguments.js; V8 and two staging/sm tests differ).
    assert_eq!(
        annex_b("(function() { arguments; { function arguments() {} } }())"),
        vec![false]
    );
    // arguments is a parameter name when the arguments object is needed.
    assert_eq!(
        annex_b("(function() { { function arguments() {} } }())"),
        vec![false]
    );
    // Not plain functions: no Annex B.
    assert_eq!(
        annex_b("function g() { { async function f() {} function* h() {} } }"),
        vec![false, false]
    );
    // `if (x) function f() {}` is a block function in a synthetic block.
    assert_eq!(
        annex_b("function g(x) { if (x) function f() {} }"),
        vec![true]
    );
}

#[test]
fn if_function_gets_a_synthetic_block() {
    let src = "function g(x) { if (x) function f() {} else function h() {} }";
    let a = script(src);
    let at = u32::try_from(src.find("function f").unwrap()).unwrap();
    let block = a.synthetic_block_at(at).expect("a synthetic block");
    assert_eq!(a.scope(block).kind, ScopeKind::Block);
    assert_eq!(a.binding(a.named("f")[0]).scope, block);
}

// arguments.

#[test]
fn arguments_object_kinds() {
    // Sloppy, simple: mapped, parameters alias it (context slots).
    let a = script("function f(a) { return arguments; }");
    let f = a.func(1);
    assert!(f.mapped_arguments);
    assert_eq!(ctx(&a, "a").1, ContextReason::MappedArguments);
    assert_eq!(a.the("arguments").kind, BindingKind::Arguments);
    assert!(is_register(a.loc("arguments")));
    // Strict or non-simple: unmapped, parameters stay where they are.
    let a = strict("function f(a) { return arguments; }");
    assert!(!a.func(1).mapped_arguments && a.func(1).arguments.is_some());
    assert_eq!(a.loc("a"), BindingLocation::Parameter(0));
    let a = script("function f(a = 1) { return arguments; }");
    assert!(!a.func(1).mapped_arguments && a.func(1).arguments.is_some());
    // Not used: no object.
    let a = script("function f(a) { return a; }");
    assert_eq!(a.func(1).arguments, None);
}

#[test]
fn arguments_resolution_rules() {
    // Arrows use the enclosing function's object.
    let a = script("function f() { return () => arguments; }");
    assert_eq!(ctx(&a, "arguments").1, ContextReason::Captured);
    assert_eq!(a.func(2).arguments, None);
    // A parameter named arguments wins.
    let a = script("function f(arguments) { return arguments; }");
    assert_eq!(a.the("arguments").kind, BindingKind::Parameter);
    assert_eq!(a.func(1).arguments, None);
    // `var arguments` shares the binding with the object.
    let a = script("function f() { var arguments; return arguments; }");
    assert_eq!(a.func(1).arguments, Some(a.id_of("arguments")));
    assert_eq!(a.the("arguments").kind, BindingKind::Var);
    // A function declaration named arguments replaces it.
    let a = script("function f() { function arguments() {} return arguments; }");
    assert_eq!(a.func(1).arguments, None);
    assert_eq!(a.the("arguments").kind, BindingKind::Function);
    // ... but not when parameters have expressions (the declaration is in the body scope).
    let a = script("function f(p = 1) { let arguments; }");
    assert_eq!(a.named("arguments").len(), 1);
    // Script code: a global name.
    assert_eq!(script("arguments;").res("arguments"), Resolution::Global);
}

// Parameters and body split (FunctionDeclarationInstantiation step 28).

#[test]
fn parameter_closures_do_not_see_body_vars() {
    // node 24: `k()` throws ReferenceError (x is not defined): the closure sees no body var.
    let a = script("function k(a = () => x) { var x = 1; return a(); }");
    let f = a.func(1);
    assert!(f.has_parameter_expressions);
    assert_ne!(f.scope, f.body_scope);
    assert_eq!(a.scope(f.body_scope).kind, ScopeKind::FunctionBody);
    assert_eq!(a.the("x").scope, f.body_scope);
    let xs = a.refs("x");
    assert_eq!(xs[0].resolution, Resolution::Global, "{a}");
}

#[test]
fn body_vars_start_with_the_parameter_value() {
    let a = script("function f(a = 1) { var a; return a; }");
    let all = a.named("a");
    assert_eq!(all.len(), 2);
    assert_eq!(a.binding(all[1]).init_from, Some(all[0]));
    assert_eq!(a.target("a", 0), all[1]);
    let a = script("function f(p = 1) { var arguments; return arguments; }");
    let args = a.named("arguments");
    assert_eq!(args.len(), 2, "{a}");
    let var = args
        .iter()
        .copied()
        .find(|&b| a.binding(b).kind == BindingKind::Var)
        .unwrap();
    let object = args
        .iter()
        .copied()
        .find(|&b| a.binding(b).kind == BindingKind::Arguments)
        .unwrap();
    assert_eq!(a.binding(var).init_from, Some(object));
}

// this, new.target, super.

#[test]
fn arrows_capture_this_new_target_and_super() {
    let a = script(
        "class A extends Object {
           constructor() { const f = () => super(); f(); }
           m() { return () => super.m() + this.x + new.target; }
         }",
    );
    let ctor = a
        .functions()
        .iter()
        .find(|f| f.kind == FunctionKind::DerivedConstructor)
        .unwrap();
    for b in [ctor.callee, ctor.new_target, ctor.this] {
        let b = b.expect("used by the arrow");
        assert!(
            matches!(a.binding(b).location, BindingLocation::Context { .. }),
            "{a}"
        );
    }
    // `this` of a derived constructor: always a hole check.
    let this_refs: Vec<_> = a
        .references()
        .iter()
        .filter(|r| r.kind == RefKind::This)
        .collect();
    assert_eq!(
        this_refs[0].resolution,
        Resolution::Binding {
            binding: ctor.this.unwrap(),
            hops: 0,
            hole_check: true
        }
    );
    let m = a
        .functions()
        .iter()
        .find(|f| f.kind == FunctionKind::Method)
        .unwrap();
    let home = m.home_object.expect("super.m()");
    assert_eq!(
        a.binding(home).context_reason,
        Some(ContextReason::Captured)
    );
}

#[test]
fn uncaptured_this_and_home_object_stay_in_the_frame() {
    let a = script("function f() { return this; } ({ m() { return super.x; } });");
    let f = a.func(1);
    assert_eq!(
        a.binding(f.this.unwrap()).location,
        BindingLocation::Frame(FrameSlot::Receiver)
    );
    let m = a.func(2);
    assert_eq!(m.kind, FunctionKind::Method);
    assert_eq!(
        a.binding(m.home_object.unwrap()).location,
        BindingLocation::Frame(FrameSlot::HomeObject)
    );
    // Script `this` captured by a top-level arrow: a context slot in the script scope.
    let a = script("() => this;");
    assert_eq!(ctx(&a, "this").0, ScopeId(0));
}

// Classes.

#[test]
fn class_scopes_private_names_and_synthetic_functions() {
    let a = script(
        "class C extends (function h() {}) {
           #x = 1; static #y; y = () => this.#x;
           constructor() { super(); }
           m() { return C.#y; }
           static { var z = this; }
         }",
    );
    // Function ids (a preorder of the parent links): script, h (heritage), the instance
    // initializer and the arrow in field `y` (its child), the static initializer, then the
    // methods in source order: the constructor, m.
    let kinds: Vec<_> = a.functions().iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![
            FunctionKind::Script,
            FunctionKind::Normal,
            FunctionKind::InstanceMembersInitializer,
            FunctionKind::Arrow,
            FunctionKind::StaticMembersInitializer,
            FunctionKind::DerivedConstructor,
            FunctionKind::Method,
        ]
    );
    assert_eq!(a.func(3).parent, Some(FunctionId(2)));
    let class_scope = a
        .scopes()
        .iter()
        .find(|s| s.kind == ScopeKind::Class)
        .unwrap();
    let info = class_scope.class.as_ref().unwrap();
    assert!(info.derived);
    assert_eq!(info.constructor, Some(FunctionId(5)));
    assert_eq!(info.default_constructor, None);
    assert_eq!(info.instance_initializer, Some(FunctionId(2)));
    assert_eq!(info.static_initializer, Some(FunctionId(4)));
    // Private names and the captured inner name live in the class context.
    for name in ["#x", "#y"] {
        assert_eq!(ctx(&a, name).0, class_scope.id);
    }
    let inner = info.inner_name.unwrap();
    assert!(matches!(
        a.binding(inner).location,
        BindingLocation::Context { .. }
    ));
    // The outer binding is a global lexical.
    let outer = a.named("C").into_iter().find(|&b| b != inner).unwrap();
    assert_eq!(a.binding(outer).location, BindingLocation::GlobalLexical);
    // The arrow in the field captures `this` of the instance initializer.
    let init = a.func(2);
    assert_eq!(
        a.binding(init.this.unwrap()).context_reason,
        Some(ContextReason::Captured)
    );
    // The static block's var is local to the static initializer.
    let z = a.the("z");
    assert_eq!(a.scope(z.scope).kind, ScopeKind::StaticBlock);
    assert_eq!(a.scope(z.scope).function, FunctionId(4));
    assert!(a.func(4).this.is_some());
}

#[test]
fn default_constructors_are_synthetic() {
    let a = script("class A {} class B extends A {}");
    let kinds: Vec<_> = a.functions().iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![
            FunctionKind::Script,
            FunctionKind::DefaultConstructor,
            FunctionKind::DefaultDerivedConstructor
        ]
    );
}

#[test]
fn private_getter_and_setter_share_a_binding() {
    let a = script("class C { get #a() { return 1; } set #a(v) {} m() { return this.#a; } }");
    assert_eq!(
        a.the("#a").kind,
        BindingKind::PrivateName {
            kind: PrivateKind::GetterSetter,
            is_static: false
        }
    );
}

#[test]
fn computed_keys_and_heritage_run_in_the_enclosing_function() {
    let a = script("function f(k) { return class extends k { [k]() {} }; }");
    for r in a.refs("k") {
        assert_eq!(r.function, FunctionId(1));
        assert!(matches!(r.resolution, Resolution::Binding { hops: 0, .. }));
    }
    assert_eq!(a.loc("k"), BindingLocation::Parameter(0));
}

// Catch, for, function names.

#[test]
fn catch_parameters_and_var_redeclaration() {
    let a = script("function f() { try {} catch (e) { var e = 1; () => e; } }");
    let all = a.named("e");
    assert_eq!(all.len(), 2);
    let catch = all
        .iter()
        .copied()
        .find(|&b| a.binding(b).kind == BindingKind::CatchParameter)
        .unwrap();
    let var = all
        .iter()
        .copied()
        .find(|&b| a.binding(b).kind == BindingKind::Var)
        .unwrap();
    assert_eq!(a.binding(var).scope, a.func(1).scope);
    // `var e = 1` assigns the catch parameter (B.3.4).
    assert_eq!(a.target("e", 0), catch);
    assert_eq!(
        a.binding(catch).context_reason,
        Some(ContextReason::Captured)
    );
    assert_eq!(a.scope(a.binding(catch).scope).kind, ScopeKind::Catch);
}

#[test]
fn for_let_bindings_are_per_iteration() {
    let a = script("var fs = []; for (let i = 0; i < 3; i++) { fs.push(() => i); }");
    let (scope, _) = ctx(&a, "i");
    assert_eq!(a.scope(scope).kind, ScopeKind::For);
    assert_eq!(a.scope(scope).per_iteration, PerIteration::Copy);
    let a = script("for (const x of []) { () => x; }");
    let (scope, _) = ctx(&a, "x");
    assert_eq!(a.scope(scope).per_iteration, PerIteration::Fresh);
    let a = script("for (var v = 0; v < 1; v++) {}");
    assert_eq!(a.loc("v"), BindingLocation::Global);
}

#[test]
fn named_function_expressions() {
    let a = script("var f = function g() { return g; };");
    let g = a.the("g");
    assert_eq!(g.kind, BindingKind::FunctionName);
    assert_eq!(g.location, BindingLocation::Frame(FrameSlot::Callee));
    assert_eq!(a.scope(g.scope).kind, ScopeKind::FunctionName);
    // Captured: a context slot in the name scope (outside the parameters).
    let a = script("var f = function g() { return () => g; };");
    let (scope, _) = ctx(&a, "g");
    assert_eq!(a.scope(scope).kind, ScopeKind::FunctionName);
    assert!(a.scope(scope).needs_context);
    // A parameter or var of the same name shadows it.
    let a = script("(function g(g) { return g; });");
    assert_eq!(a.binding(a.target("g", 0)).kind, BindingKind::Parameter);
}

// Determinism, depth.

#[test]
fn equal_input_gives_equal_output() {
    let src = "var a; function f(x, y = () => x) { let z; with (y) { eval(''); } class K { #p; } return () => z; }";
    let first = script(src);
    for _ in 0..3 {
        assert_eq!(script(src), first);
    }
    assert_eq!(script(src).to_string(), first.to_string());
}

#[test]
fn layer_3_reports_a_range_error() {
    let src = format!("x = {}0{};", "[".repeat(50), "]".repeat(50));
    let result = with_front_end(&src, SourceOpts::script(Strictness::Sloppy), |fe| {
        analyze_with_cap(fe, 20)
    });
    let err = result.unwrap_err();
    let range = err.as_range().expect("a RangeError");
    assert_eq!(range.kind, crate::RangeErrorKind::TooDeep(DepthLayer::Pass));
    // With the real cap the same program is fine.
    assert!(script(&src).functions().len() == 1);
}

// Cross-check with oxc.

/// Why ep9 and oxc resolve a reference to different declarations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Why {
    /// Parameters and body split: ep9 reads the body `var`, initialized from the parameter.
    SplitBody,
    /// Annex B: oxc moves every block function to the var scope, even when B.3.2 skips it,
    /// and merges it with an existing `var`/function (one symbol declared at the first site).
    AnnexB,
    /// oxc merges a `catch (e)` parameter and a `var e` in its block into one symbol.
    CatchVar,
    /// Not explained: a bug in one of the two.
    Other,
}

/// One reference where ep9 and oxc resolve to different declarations.
#[derive(Debug)]
pub(crate) struct Mismatch {
    pub(crate) at: u32,
    pub(crate) name: String,
    pub(crate) ep9: Option<u32>,
    pub(crate) oxc: Option<u32>,
    pub(crate) why: Why,
}

/// Compare every `IdentifierReference`: ep9's binding (or the fallback of a dynamic lookup)
/// must be declared at the same identifier as oxc's symbol. Implicit `arguments` and global
/// names match "unresolved" in oxc. Also returns how many oxc references and oxc scopes have
/// no ep9 counterpart (must be 0: BytecodeGen looks them up).
pub(crate) fn oxc_mismatches(fe: &FrontEnd<'_>, a: &ScopeAnalysis) -> (Vec<Mismatch>, usize) {
    let scoping = fe.semantic.scoping();
    let mut mismatches = Vec::new();
    let mut unvisited = 0;
    for i in 0..scoping.references_len() {
        let id = ReferenceId::new(i);
        let Some(mine) = a.reference(id) else {
            unvisited += 1;
            continue;
        };
        let symbol = scoping.get_reference(id).symbol_id();
        let theirs = symbol.map(|s| scoping.symbol_span(s).start);
        let binding = match mine.resolution {
            Resolution::Binding { binding, .. }
            | Resolution::Dynamic {
                fallback: Some(binding),
            } => Some(binding),
            Resolution::Global | Resolution::Dynamic { fallback: None } => None,
        };
        let ours = binding.and_then(|b| a.binding(b).span).map(|s| s.start);
        if ours != theirs {
            let kind_at =
                |start: Option<u32>| start.and_then(|s| a.declared_at(s)).map(|b| a.binding(b));
            let (ep9_binding, oxc_binding) = (binding.map(|b| a.binding(b)), kind_at(theirs));
            let why = if ep9_binding.is_some_and(|b| b.init_from.is_some())
                || ep9_binding.is_some_and(|b| a.scope(b.scope).kind == ScopeKind::FunctionBody)
                    && oxc_binding.is_some_and(|b| b.kind == BindingKind::Parameter)
            {
                Why::SplitBody
            } else if oxc_binding.is_some_and(|b| b.kind == BindingKind::BlockFunction)
                // oxc hoists a block function that B.3.2 skips (and may merge it with a `let`).
                || a.bindings().iter().any(|b| {
                    b.kind == BindingKind::BlockFunction && b.name == mine.name && b.annex_b_var.is_none()
                })
                || ep9_binding.is_some_and(|b| b.kind == BindingKind::BlockFunction || b.span.is_some_and(|s| a.declared_at(s.start).is_some_and(|d| a.binding(d).annex_b_var.is_some())))
            {
                Why::AnnexB
            } else if symbol.is_some_and(|s| {
                scoping
                    .symbol_flags(s)
                    .contains(oxc_semantic::SymbolFlags::CatchVariable)
            }) {
                Why::CatchVar
            } else {
                Why::Other
            };
            mismatches.push(Mismatch {
                at: mine.span.start,
                name: a.name(mine.name).to_owned(),
                ep9: ours,
                oxc: theirs,
                why,
            });
        }
    }
    // Every oxc scope node has an ep9 scope (BytecodeGen looks scopes up by oxc scope id).
    for i in 0..scoping.scopes_len() {
        if a.scope_for(oxc_semantic::ScopeId::new(i)).is_none() {
            unvisited += 1;
        }
    }
    (mismatches, unvisited)
}

fn cross_check(src: &str, opts: SourceOpts) -> (Vec<Mismatch>, usize) {
    with_front_end(src, opts, |fe| {
        let a = analyze(fe)?;
        Ok(oxc_mismatches(fe, &a))
    })
    .unwrap()
}

#[test]
fn bytecodegen_lookups() {
    let src = "function f(o) { var v = this; { let b; } return [new.target, super.x, o.#p]; }";
    // `super` and `#p` outside a class are early errors, so check those in a class.
    let class_src =
        "class C extends Object { #p; constructor() { super(); } m(o) { return super.x + o.#p; } }";
    for (src, ok) in [(src, false), (class_src, true)] {
        let result = with_front_end(src, SourceOpts::script(Strictness::Sloppy), move |fe| {
            let a = analyze(fe)?;
            let scoping = fe.semantic.scoping();
            // Program scope -> root; every oxc scope maps to an ep9 scope.
            assert_eq!(a.scope_for(oxc_semantic::ScopeId::new(0)), Some(ScopeId(0)));
            for i in 0..scoping.scopes_len() {
                assert!(a.scope_for(oxc_semantic::ScopeId::new(i)).is_some());
            }
            let at = |needle: &str| u32::try_from(src.find(needle).unwrap()).unwrap();
            let sup = at("super(");
            for key in [
                ImplicitRef::Callee,
                ImplicitRef::NewTarget,
                ImplicitRef::This,
            ] {
                assert!(a.implicit(sup, key).is_some(), "{key:?}");
            }
            assert!(a.implicit(sup, ImplicitRef::HomeObject).is_none());
            let sup_x = at("super.x");
            assert_eq!(
                a.implicit(sup_x, ImplicitRef::HomeObject).unwrap().kind,
                RefKind::HomeObject
            );
            let private = at("#p;")
                + 1
                + u32::try_from(src[at("#p;") as usize + 1..].find("#p").unwrap()).unwrap();
            assert_eq!(
                a.implicit(private, ImplicitRef::Private).unwrap().kind,
                RefKind::PrivateName
            );
            Ok(())
        });
        assert_eq!(result.is_ok(), ok, "{src}: {result:?}");
    }
    let src = "function f() { var v = this; return new.target; }";
    with_front_end(src, SourceOpts::script(Strictness::Sloppy), move |fe| {
        let a = analyze(fe)?;
        let at = |needle: &str| u32::try_from(src.find(needle).unwrap()).unwrap();
        let this = a.implicit(at("this"), ImplicitRef::This).unwrap();
        assert!(matches!(this.resolution, Resolution::Binding { .. }));
        assert!(
            a.implicit(at("new.target"), ImplicitRef::NewTarget)
                .is_some()
        );
        let var = a.implicit(at("v ="), ImplicitRef::VarInit).unwrap();
        assert_eq!(var.kind, RefKind::VarInit);
        assert_eq!(a.declared_at(at("v =")), Some(var_binding(&a, var)));
        assert_eq!(a.function_at(at("function f")), Some(FunctionId(1)));
        Ok(())
    })
    .unwrap();
}

fn var_binding(a: &ScopeAnalysis, r: &Reference) -> BindingId {
    match r.resolution {
        Resolution::Binding { binding, .. } => binding,
        other => panic!("{other:?}\n{a}"),
    }
}

#[test]
fn agrees_with_oxc_where_the_trees_match() {
    let programs = [
        "var a = 1; function f(b) { let c = a + b; { const d = c; return () => d + c; } }",
        "try { x } catch (e) { e; var e2 = e; } for (let i of []) i; for (var j in {}) j;",
        "label: for (;;) { break label; } switch (1) { case 1: let s = 2; s; }",
        "class C extends Object { static m() { return C; } #p = 1; q() { return this.#p; } }",
        "var g = function h() { return h; }; (function () { return arguments; })();",
        "function k() { { function blk() {} } blk(); }",
        "with ({}) { undefinedName; } eval('1');",
        "({ m() { return super.toString(); }, get x() { return 1; }, [Symbol.iterator]: 1 });",
        "async function af() { await 0; for await (const v of []) v; } function* gf() { yield 1; }",
    ];
    for src in programs {
        let (mismatches, unvisited) = cross_check(src, SourceOpts::script(Strictness::Sloppy));
        assert!(mismatches.is_empty(), "{src}\n{mismatches:?}");
        assert_eq!(unvisited, 0, "{src}");
    }
    let (mismatches, unvisited) = cross_check(
        "import d from 'm'; export const e = d; let n = 1; export { n as m2 }; export default function () { return n; }",
        SourceOpts::module(),
    );
    assert!(mismatches.is_empty(), "{mismatches:?}");
    assert_eq!(unvisited, 0);
}

/// Evidence for the module doc table: where oxc's scope tree differs from the ECMA-262
/// environments, and what ep9 builds instead. If an oxc bump changes one of these, this test
/// says so (then the table needs an update, not the pass).
#[test]
fn oxc_scope_tree_differs_from_the_spec() {
    fn oxc_tree(src: &str) -> Vec<(String, u32)> {
        // (name, oxc scope id) of every oxc symbol.
        with_front_end(src, SourceOpts::script(Strictness::Sloppy), |fe| {
            let s = fe.semantic.scoping();
            Ok((0..s.symbols_len())
                .map(|i| {
                    let id = oxc_semantic::SymbolId::new(i);
                    (
                        s.symbol_name(id).to_owned(),
                        u32::try_from(s.symbol_scope_id(id).index()).unwrap(),
                    )
                })
                .collect())
        })
        .unwrap()
    }
    let scope_of = |tree: &[(String, u32)], name: &str| {
        tree.iter()
            .find(|(n, _)| n == name)
            .map(|&(_, s)| s)
            .unwrap()
    };

    // 1. Parameters and body: one oxc scope; two in ep9 (oxc still resolves `x` right).
    let src = "function k(a = () => x) { var x = 1; }";
    let tree = oxc_tree(src);
    assert_eq!(scope_of(&tree, "a"), scope_of(&tree, "x"));
    let a = script(src);
    assert_ne!(a.the("a").scope, a.the("x").scope);
    assert!(
        cross_check(src, SourceOpts::script(Strictness::Sloppy))
            .0
            .is_empty()
    );

    // 2. Class declaration: oxc has only the outer binding; a method sees it.
    let src = "class C { m() { return C; } }";
    let tree = oxc_tree(src);
    assert_eq!(tree.iter().filter(|(n, _)| n == "C").count(), 1);
    assert_eq!(scope_of(&tree, "C"), 0);
    let a = script(src);
    assert_eq!(a.binding(a.target("C", 0)).kind, BindingKind::ClassInner);

    // 3. Named function expression: oxc puts the name in the function scope with the
    //    parameters; ep9 has a name scope outside them.
    let src = "var f = function g(p) { return g; };";
    let tree = oxc_tree(src);
    assert_eq!(scope_of(&tree, "g"), scope_of(&tree, "p"));
    let a = script(src);
    assert_ne!(a.the("g").scope, a.the("p").scope);

    // 4. Annex B: oxc moves the block function to the function scope; ep9 keeps a block
    //    binding and adds a var binding.
    let src = "function h() { { function b() {} } }";
    let tree = oxc_tree(src);
    assert_eq!(scope_of(&tree, "b"), scope_of(&tree, "h") + 1);
    let a = script(src);
    let kinds: Vec<_> = a.named("b").iter().map(|&b| a.binding(b).kind).collect();
    assert_eq!(kinds, vec![BindingKind::BlockFunction, BindingKind::Var]);
}

#[test]
#[ignore = "debug dump"]
fn dump_one() {
    let src = std::env::var("EP9_SRC").unwrap();
    println!("{}", script(&src));
}
