//! Math (§21.3): the constants, `[@@toStringTag]`, abs, ceil, floor, fround, max, min, pow,
//! round, sign, sqrt and trunc.
//!
//! The expected lines are node 24's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn acceptance_math_pow_2_32() {
    assert_eq!(
        common::run_both(r#"print(Math.pow(2,32)-1===4294967295)"#),
        lines(&["true"])
    );
}

#[test]
fn math_is_an_object_with_a_tag() {
    assert_eq!(
        common::run_both(
            r#"print(typeof Math, Math[Symbol.toStringTag], String(Math) === '[object Math]' || String(Math) === '[object Object]'); try { Math(); } catch (e) { print(e.name); } try { new Math(); } catch (e) { print(e.name); }"#
        ),
        lines(&["object Math true", "TypeError", "TypeError"])
    );
}

#[test]
fn math_constants() {
    assert_eq!(
        common::run_both(
            r#"print(Math.E, Math.LN10, Math.LN2, Math.LOG10E, Math.LOG2E, Math.PI, Math.SQRT1_2, Math.SQRT2)"#
        ),
        lines(&[
            "2.718281828459045 2.302585092994046 0.6931471805599453 0.4342944819032518 1.4426950408889634 3.141592653589793 0.7071067811865476 1.4142135623730951"
        ])
    );
}

#[test]
fn math_constants_are_read_only() {
    assert_eq!(
        common::run_both(r#"Math.PI = 3; delete Math.E; print(Math.PI, Math.E)"#),
        lines(&["3.141592653589793 2.718281828459045"])
    );
}

#[test]
fn abs_ceil_floor_trunc() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(z(Math.abs(-0)), Math.abs(-Infinity), Math.abs('-2'), z(Math.ceil(-0.5)), Math.ceil(1.1), z(Math.floor(0.5)), Math.floor(-1.1), z(Math.trunc(-0.9)), Math.trunc(2.9), Math.abs(), Math.ceil(NaN))"#
        ),
        lines(&["+0 Infinity 2 -0 2 +0 -2 -0 2 NaN NaN"])
    );
}

#[test]
fn sign_sqrt() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.sign(-3), Math.sign(4), z(Math.sign(-0)), Math.sign(NaN), Math.sqrt(2), Math.sqrt(-1), z(Math.sqrt(-0)), Math.sqrt(Infinity), Math.sqrt(16))"#
        ),
        lines(&["-1 1 -0 NaN 1.4142135623730951 NaN -0 Infinity 4"])
    );
}

#[test]
fn round_and_fround() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.round(2.5), Math.round(-2.5), z(Math.round(-0.4)), Math.round(0.49999999999999994), Math.round(4503599627370495.5), Math.fround(5.05), Math.fround(0.1), z(Math.fround(-1e-46)), Math.fround(1e39), Math.round('3.7'))"#
        ),
        lines(&["3 -2 -0 0 4503599627370496 5.050000190734863 0.10000000149011612 -0 Infinity 4"])
    );
}

#[test]
fn max_and_min() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.max(), Math.min(), Math.max(1, 3, 2), Math.min(1, -3, 2), Math.max(1, NaN, 3), Math.min(NaN), z(Math.max(-0, 0)), z(Math.max(0, -0)), z(Math.min(0, -0)), z(Math.min(-0, 0)), Math.max('7', 6))"#
        ),
        lines(&["-Infinity Infinity 3 -3 NaN NaN +0 +0 -0 -0 7"])
    );
}

#[test]
fn max_converts_every_argument_before_nan() {
    assert_eq!(
        common::run_both(
            r#"var log = []; var r = Math.max(NaN, { valueOf: function () { log.push('a'); return 1; } }, { valueOf: function () { log.push('b'); return 2; } }); print(r, log.join())"#
        ),
        lines(&["NaN a,b"])
    );
}

#[test]
fn pow_exact_and_special_cases() {
    assert_eq!(
        common::run_both(
            r#"print(Math.pow(2, 10), Math.pow(2, -1), Math.pow(NaN, 0), Math.pow(-8, 1 / 3), Math.pow(2, 0.5), Math.pow(1.5, 2), Math.pow(-2, 3), Math.pow(1, Infinity), Math.pow(0, -1), Math.pow(-0, -3), Math.pow('3', '2'), Math.pow())"#
        ),
        lines(&["1024 0.5 1 NaN 1.4142135623730951 2.25 -8 NaN Infinity -Infinity 9 NaN"])
    );
}

#[test]
fn math_function_lengths_and_names() {
    assert_eq!(
        common::run_both(
            r#"print(Math.abs.length, Math.max.length, Math.min.length, Math.pow.length, Math.round.name, Math.fround.length, Math.trunc.name)"#
        ),
        lines(&["1 2 2 2 round 1 trunc"])
    );
}

/// v0: where the result is not exact, `Math.pow` needs the correctly rounded pow core (H36,
/// month 2). Until then it throws the catchable "not implemented" Error of `**`, with the
/// same text: both are Number::exponentiate (`ep9_math::pow_js` gives `None`).
#[test]
fn pow_without_an_exact_result_is_not_implemented_like_exponentiation() {
    const TEXT: &str = "not implemented in ep9 v0: ** with a result that is not an exact \
                        integer: needs the correctly rounded pow core (H36, month 2)";
    assert_eq!(
        common::run_both(
            r#"var x = 3, y = 0.3, a, b; try { Math.pow(x, y); } catch (e) { a = e.message; } try { x ** y; } catch (e) { b = e.message; } print(a === b, a)"#
        ),
        Ok(vec![format!("true {TEXT}")])
    );
    assert_eq!(
        common::run_both("Math.pow(3, 0.3)"),
        Err(format!("Error: {TEXT}"))
    );
}
