//! Depth guard gate tests (M0 exit item 7, M1 exit item 8): for every nesting form, the deepest
//! input at the cap compiles and one level more is a RangeError, through the public API, in the
//! build profile under test. The test process staying alive is the "no abort" check.

use crate::forms::{FORMS, Form, Gate};
use crate::prescan::{self, REGEX_WEIGHT};
use crate::{
    AST_CAP, CompileError, DepthLayer, PRESCAN_CAP, RangeErrorKind, SourceOpts, Strictness,
    check_early_errors, depth, parse,
};
use oxc_allocator::Allocator;

fn form(name: &str) -> &'static Form {
    FORMS
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no form {name}"))
}

fn opts(form: &Form) -> SourceOpts {
    if form.module {
        SourceOpts::module()
    } else {
        SourceOpts::script(Strictness::Sloppy)
    }
}

/// AST depth of a small instance (on the test thread; small n only).
fn ast_depth(form: &Form, n: usize) -> u32 {
    let src = (form.build)(n);
    let allocator = Allocator::default();
    let program = parse(&allocator, &src, opts(form)).expect("small instances parse");
    depth::check_ast_depth(&program, u32::MAX).expect("no cap")
}

fn estimate(form: &Form, n: usize) -> u32 {
    prescan::estimate((form.build)(n).as_bytes(), form.module).depth
}

/// `(a, b)` with `depth(n) = a * n + b`, checked on three points.
fn linear(f: impl Fn(usize) -> u32) -> (u32, u32) {
    let (d10, d11, d17) = (f(10), f(11), f(17));
    let a = d11 - d10;
    let b = d10 - 10 * a;
    assert_eq!(d17, 17 * a + b, "not linear: {d10} {d11} {d17}");
    (a, b)
}

fn layer_of(result: &Result<(), CompileError>) -> Option<DepthLayer> {
    match result {
        Err(CompileError::Range(e)) => match e.kind {
            RangeErrorKind::TooDeep(layer) => Some(layer),
            RangeErrorKind::StackUnavailable => panic!("stack unavailable: {e}"),
        },
        _ => None,
    }
}

fn run(form: &Form, n: usize) -> Result<(), CompileError> {
    check_early_errors(&(form.build)(n), opts(form))
}

/// The gate check for one form whose depth grows with the AST.
fn check_ast_form(name: &str) {
    let form = form(name);
    let (a, b) = linear(|n| ast_depth(form, n));
    assert!(a >= 1, "{name}: depth does not grow");
    // The largest n with depth(n) <= AST_CAP.
    let n_cap = usize::try_from((AST_CAP - b) / a).unwrap();
    let depth_at = |n: usize| a * u32::try_from(n).unwrap() + b;
    assert!(depth_at(n_cap) <= AST_CAP && depth_at(n_cap + 1) > AST_CAP);

    // The pre-scan must never reject what layer 2 accepts.
    let (ea, eb) = linear(|n| estimate(form, n));
    let est_cap = ea * u32::try_from(n_cap).unwrap() + eb;
    assert!(
        est_cap <= PRESCAN_CAP,
        "{name}: estimate {est_cap} > PRESCAN_CAP at AST depth {}",
        depth_at(n_cap)
    );

    // cap - 1 (or the instance just below the cap) compiles.
    let below = run(form, n_cap - 1);
    assert!(
        below.is_ok(),
        "{name} n={} depth {}: {below:?}",
        n_cap - 1,
        depth_at(n_cap - 1)
    );
    // The deepest instance at or below the cap compiles.
    let at = run(form, n_cap);
    assert!(
        at.is_ok(),
        "{name} n={n_cap} depth {}: {at:?}",
        depth_at(n_cap)
    );
    // The scope pass (layer 3) also handles the deepest accepted tree on the worker stack.
    let scopes = crate::analyze(&(form.build)(n_cap), opts(form));
    assert!(
        scopes.is_ok(),
        "{name} n={n_cap}: scope pass {:?}",
        scopes.err()
    );
    // One more level: RangeError from layer 2 (the exact cut).
    let above = run(form, n_cap + 1);
    assert_eq!(
        layer_of(&above),
        Some(DepthLayer::Ast),
        "{name} n={} depth {}: {above:?}",
        n_cap + 1,
        depth_at(n_cap + 1)
    );
    // Far beyond: RangeError from layer 0 (or layer 2 for loops the parser builds iteratively).
    let far = run(form, 10 * n_cap);
    assert!(layer_of(&far).is_some(), "{name} n={}: {far:?}", 10 * n_cap);
    if ea > 0 {
        assert_eq!(layer_of(&far), Some(DepthLayer::PreScan), "{name}");
    }
}

/// Regex nesting has no AST depth: the pre-scan cap is the limit.
fn check_regex_form(name: &str) {
    let form = form(name);
    let (ea, eb) = linear(|n| estimate(form, n));
    assert_eq!(ea, REGEX_WEIGHT, "{name}: estimate per level");
    let n_cap = usize::try_from((PRESCAN_CAP - eb) / ea).unwrap();
    assert!(run(form, n_cap).is_ok(), "{name} n={n_cap}");
    let above = run(form, n_cap + 1);
    assert_eq!(
        layer_of(&above),
        Some(DepthLayer::PreScan),
        "{name}: {above:?}"
    );
    // Far beyond: more levels than the worker stack holds in either profile without layer 0.
    let far = run(form, 1_000_000);
    assert_eq!(
        layer_of(&far),
        Some(DepthLayer::PreScan),
        "{name} n=1000000: {far:?}"
    );
}

macro_rules! gate {
    ($($test:ident => $form:literal,)*) => {
        $(
            #[test]
            fn $test() {
                check_ast_form($form);
            }
        )*
        /// Every form in `forms.rs` (except regex forms) has a test here.
        const GATED: &[&str] = &[$($form),*];
    };
}

gate! {
    // M0 exit item 7
    m0_bracket_array => "bracket-array",
    m0_bracket_paren => "bracket-paren",
    m0_bracket_object => "bracket-object",
    m0_bracket_block => "bracket-block",
    m0_bracket_function => "bracket-function",
    m0_binary => "binary",
    m0_member => "member",
    m0_call => "call",
    m0_comma_nested => "comma-nested",
    m0_template => "template",
    m0_tagged_template => "tagged-template",
    m0_class_body => "class-body",
    m0_class_heritage => "class-heritage",
    // M1 exit item 8
    m1_conditional => "conditional",
    m1_conditional_consequent => "conditional-consequent",
    m1_assignment => "assignment",
    m1_unary => "unary",
    m1_unary_typeof => "unary-typeof",
    m1_arrow => "arrow",
    m1_exponent => "exponent",
    // other recursive productions
    x_optional_chain => "optional-chain",
    x_new => "new",
    x_spread => "spread",
    x_if => "if",
    x_else_if => "else-if",
    x_label => "label",
    x_do_while => "do-while",
    x_for => "for",
    x_with => "with",
    x_try => "try",
    x_switch => "switch",
    x_arrow_block => "arrow-block",
    x_async_arrow => "async-arrow",
    x_array_pattern => "array-pattern",
    x_object_pattern => "object-pattern",
    x_param_pattern => "param-pattern",
    x_await => "await",
    x_yield => "yield",
    x_module_block => "module-block",
}

macro_rules! regex_gate {
    ($($test:ident => $form:literal,)*) => {
        $(
            #[test]
            fn $test() {
                check_regex_form($form);
            }
        )*
        /// Every regex form in `forms.rs` has a test here.
        const REGEX_GATED: &[&str] = &[$($form),*];
    };
}

regex_gate! {
    x_regex_group => "regex-group",
    x_regex_class_v => "regex-class-v",
    // With `v`, a class stays open after a nested class closes.
    x_regex_class_v_intersection => "regex-class-v-intersection",
    x_regex_class_v_subtraction => "regex-class-v-subtraction",
    x_regex_class_v_union => "regex-class-v-union",
    x_regex_group_u => "regex-group-u",
    x_regex_named_group => "regex-named-group",
    x_regex_ignore_group => "regex-ignore-group",
    x_regex_lookahead => "regex-lookahead",
    x_regex_lookbehind => "regex-lookbehind",
}

#[test]
fn every_form_is_gated() {
    for form in FORMS {
        let gated = if form.name.starts_with("regex-") {
            REGEX_GATED
        } else {
            GATED
        };
        assert!(gated.contains(&form.name), "{} has no gate test", form.name);
    }
    let m0 = FORMS.iter().filter(|f| f.gate == Gate::M0).count();
    let m1 = FORMS.iter().filter(|f| f.gate == Gate::M1).count();
    assert_eq!((m0, m1), (13, 7));
}

#[test]
fn a_flat_comma_list_has_no_depth() {
    // oxc keeps `a, b, c` flat (one SequenceExpression), so a long list is not a depth hazard.
    let src = format!("x = ({}0)", "0,".repeat(200_000));
    assert!(check_early_errors(&src, SourceOpts::script(Strictness::Sloppy)).is_ok());
}

#[test]
fn a_million_levels_never_abort() {
    for src in [
        "[".repeat(1_000_000),
        format!("1{}", "+1".repeat(1_000_000)),
        format!("a{}", ".b".repeat(1_000_000)),
        "(".repeat(1_000_000),
        format!("/{}/", "(".repeat(1_000_000)),
        "`${".repeat(300_000),
    ] {
        let result = check_early_errors(&src, SourceOpts::script(Strictness::Sloppy));
        assert!(result.is_err(), "{}...", &src[..20]);
    }
}

#[test]
fn range_error_reports_a_position() {
    let src = format!("let a = 1;\n{}", "[".repeat(100_000));
    let err = check_early_errors(&src, SourceOpts::script(Strictness::Sloppy)).unwrap_err();
    let range = err.as_range().expect("a range error");
    assert_eq!(range.kind, RangeErrorKind::TooDeep(DepthLayer::PreScan));
    let position = range.position.expect("a position");
    assert_eq!(position.line, 2);
    assert!(err.to_string().starts_with("2:"), "{err}");
    assert!(err.to_string().contains("RangeError"), "{err}");
}

/// Deterministic pseudo-random mixes of nesting forms near the caps: Ok or RangeError, never an
/// abort. This checks that the stack model holds for mixtures, not only for pure forms.
#[test]
fn random_mixes_near_the_caps_never_abort() {
    // Expression wrappers: (prefix, suffix). Each one yields a primary-like expression, so any
    // wrapper can hold any other (operators and arrows are parenthesized).
    const WRAP: &[(&str, &str)] = &[
        ("[", "]"),
        ("(", ")"),
        ("({a:", "})"),
        ("`${", "}`"),
        ("(!", ")"),
        ("(a=", ")"),
        ("(x=>", ")"),
        ("(a?", ":0)"),
        ("(a**", ")"),
        ("new ", ""),
        ("(0,", ")"),
        ("function(){return ", "}"),
        ("class extends ", "{}"),
        ("[...", "]"),
        ("(()=>{if(a)return ", "})"),
        ("(typeof ", ")"),
        ("(async x=>", ")"),
        ("f(", ")"),
    ];
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut outcomes = [0usize; 2];
    for round in 0..12 {
        let target = match round % 3 {
            0 => AST_CAP as usize / 2,
            1 => PRESCAN_CAP as usize - 50,
            _ => PRESCAN_CAP as usize + 50,
        };
        let mut prefix = String::from("x=");
        let mut suffix = String::new();
        for _ in 0..target {
            let (p, s) = WRAP[usize::try_from(next() % WRAP.len() as u64).unwrap()];
            prefix.push_str(p);
            suffix.insert_str(0, s);
        }
        let src = format!("{prefix}0{suffix}");
        let result = check_early_errors(&src, SourceOpts::script(Strictness::Sloppy));
        match &result {
            Ok(()) => outcomes[0] += 1,
            Err(CompileError::Range(_)) => outcomes[1] += 1,
            Err(CompileError::Syntax(e)) => {
                let at = e.first().span.map_or(0, |s| s.start as usize);
                let from = at.saturating_sub(60);
                let to = (at + 60).min(src.len());
                panic!(
                    "round {round}: unexpected syntax error {e}\n...{}...",
                    &src[from..to]
                );
            }
        }
    }
    // Some rounds compile, some hit a cap.
    assert!(outcomes[0] > 0 && outcomes[1] > 0, "{outcomes:?}");
}
