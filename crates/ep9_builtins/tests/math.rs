//! Math (§21.3): the constants, `[@@toStringTag]`, abs, ceil, clz32, exp, floor, fround, imul,
//! log, max, min, pow, random, round, sign, sqrt and trunc.
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

/// `Math.pow` and `**` give the correctly rounded result (M4-P1, the CORE-MATH pow port in
/// ep9_math; H36). node 24 prints the same digits for these inputs.
#[test]
fn pow_correctly_rounded() {
    assert_eq!(
        common::run_both(
            r#"print(Math.pow(2, 0.3), 2 ** 0.3, Math.pow(10, -1.5), 3 ** 0.3, Math.pow(Math.E, 2.5), (-1.5) ** 3, Math.pow(10, 308), Math.pow(10, 309), Math.pow(10, -323), 10 ** -5, 10 ** 22, 10 ** 23, 1.0000001 ** 1e9)"#
        ),
        lines(&[
            "1.2311444133449163 1.2311444133449163 0.03162277660168379 1.3903891703159093 12.182493960703471 -3.375 1e+308 Infinity 1e-323 0.00001 1e+22 1e+23 2.6881038582144647e+43"
        ])
    );
}

#[test]
fn exp_and_log() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.exp(1), Math.exp(-1), z(Math.exp(-Infinity)), Math.exp(Infinity), Math.exp(NaN), Math.exp(0), Math.exp(-0), Math.exp(709.7), Math.exp(710), z(Math.exp(-746)), Math.exp(-745), Math.exp('2')); print(Math.log(Math.E), Math.log(10), z(Math.log(1)), Math.log(0), Math.log(-0), Math.log(-1), Math.log(NaN), Math.log(Infinity), Math.log(5e-324), Math.log(1.7976931348623157e308), Math.log('100'))"#
        ),
        lines(&[
            "2.718281828459045 0.36787944117144233 +0 Infinity NaN 1 1 1.6549840276802644e+308 Infinity +0 5e-324 7.38905609893065",
            "1 2.302585092994046 +0 -Infinity -Infinity NaN NaN Infinity -744.4400719213812 709.782712893384 4.605170185988092",
        ])
    );
}

/// The base.js score step (GeometricMean: `Math.pow(Math.E, log / n)` over `Math.log`), on a
/// sum that node 24 prints the same way.
#[test]
fn log_exp_pow_sum() {
    assert_eq!(
        common::run_both(
            r#"var s = 0; for (var j = 1; j < 200; j++) s += Math.log(j) + Math.exp(j / 50) + Math.pow(j, 0.37); print(s)"#
        ),
        lines(&["4543.1105534202625"])
    );
}

#[test]
fn clz32_and_imul() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.clz32(1), Math.clz32(0), Math.clz32(-1), Math.clz32(0.5), Math.clz32(2 ** 32), Math.clz32(NaN), Math.clz32('8'), Math.clz32(), Math.clz32(-0x80000000)); print(Math.imul(2, 4), Math.imul(-1, 8), Math.imul(0xffffffff, 5), Math.imul(0xfffffffe, 5), z(Math.imul(-0, 7)), Math.imul(3.7, 2.9), Math.imul('3', 4), Math.imul(), Math.imul(0x7fffffff, 2)); var log = []; var r = Math.imul({ valueOf: function () { log.push('a'); return 2; } }, { valueOf: function () { log.push('b'); return 3; } }); print(r, log.join())"#
        ),
        lines(&[
            "31 32 0 32 32 32 28 32 0",
            "8 -8 -5 -10 +0 6 12 0 -2",
            "6 a,b",
        ])
    );
}

/// Math.random: numbers in [0, 1); the per-isolate xorshift128+ state is `Cx::math_random`
/// (its sequence tests are in ep9_vm `tests::layout4`).
#[test]
fn random_is_in_0_1() {
    assert_eq!(
        common::run_both(
            r#"var r = Math.random(), ok = true; for (var i = 0; i < 10000; i++) { var v = Math.random(); if (!(v >= 0 && v < 1) || typeof v !== 'number') ok = false; } print(typeof r, ok, Math.random.length, Math.random.name, Math.clz32.length, Math.imul.length, Math.exp.length, Math.log.length, Math.log.name)"#
        ),
        lines(&["number true 0 random 1 2 1 1 log"])
    );
}

/// The functions are own properties in V8's order (node 24 lists more: the rest are M4-P4).
#[test]
fn math_function_order_is_v8s() {
    assert_eq!(
        common::run_both(
            r#"print(Object.getOwnPropertyNames(Math).filter(function (k) { return typeof Math[k] === 'function'; }).join())"#
        ),
        lines(&[
            "abs,ceil,clz32,exp,floor,fround,imul,log,max,min,pow,random,round,sign,sqrt,trunc"
        ])
    );
}
