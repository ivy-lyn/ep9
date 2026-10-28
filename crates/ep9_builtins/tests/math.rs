//! Math (§21.3): the constants, `[@@toStringTag]` and every function but f16round.
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

/// The functions are own properties in V8's order (node 24 also has f16round, not built yet).
#[test]
fn math_function_order_is_v8s() {
    assert_eq!(
        common::run_both(
            r#"print(Object.getOwnPropertyNames(Math).filter(function (k) { return typeof Math[k] === 'function'; }).join())"#
        ),
        lines(&[
            "abs,acos,acosh,asin,asinh,atan,atanh,atan2,ceil,cbrt,expm1,clz32,cos,cosh,exp,floor,fround,hypot,imul,log,log1p,log2,log10,max,min,pow,random,round,sign,sin,sinh,sqrt,tan,tanh,trunc"
        ])
    );
}

/// Math.sin, Math.cos and Math.tan: correctly rounded (CORE-MATH ports in ep9_math, M4-P4;
/// H36), with the reduction of huge arguments.
#[test]
fn sin_cos_tan() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.sin(1), Math.sin(-1e-300), z(Math.sin(-0)), Math.sin(Math.PI), Math.sin(1e22), Math.sin(Infinity), Math.sin(NaN), Math.sin('2'), Math.sin(1.7976931348623157e308)); print(Math.cos(1), Math.cos(-0), Math.cos(Math.PI), Math.cos(Math.PI / 2), Math.cos(1e22), Math.cos(-Infinity), Math.cos(1e-10), Math.cos(5e-324)); print(Math.tan(1), z(Math.tan(-0)), Math.tan(Math.PI / 4), Math.tan(Math.PI / 2), Math.tan(1e22), Math.tan(Infinity), Math.tan(-3))"#
        ),
        lines(&[
            "0.8414709848078965 -1e-300 -0 1.2246467991473532e-16 -0.8522008497671888 NaN NaN 0.9092974268256817 0.004961954789184062",
            "0.5403023058681398 1 -1 6.123233995736766e-17 0.523214785395139 NaN 1 1",
            "1.5574077246549023 -0 0.9999999999999999 16331239353195370 -1.6287782256068988 NaN 0.1425465430742778",
        ])
    );
}

/// Math.asin, Math.acos, Math.atan and Math.atan2, with the atan2 cases of ±0 and ±∞.
#[test]
fn asin_acos_atan_atan2() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.asin(0.5), z(Math.asin(-0)), Math.asin(1), Math.asin(-1), Math.asin(1.0000000000000002), Math.asin(1e-200)); print(Math.acos(0.5), z(Math.acos(1)), Math.acos(-1), Math.acos(0), Math.acos(-1.5), Math.acos(0.9999999999999999)); print(Math.atan(1), z(Math.atan(-0)), Math.atan(Infinity), Math.atan(-Infinity), Math.atan(1e300), Math.atan(-0.5)); print(Math.atan2(1, 1), z(Math.atan2(0, 0)), z(Math.atan2(-0, 0)), Math.atan2(0, -0), Math.atan2(-0, -0), Math.atan2(1, -0), Math.atan2(-1, 0), Math.atan2(Infinity, -Infinity), Math.atan2(-Infinity, Infinity), z(Math.atan2(-1, Infinity)), Math.atan2(1, -Infinity), Math.atan2(NaN, 1), Math.atan2(3, -4), Math.atan2(-1e-300, 1e300), Math.atan2.length)"#
        ),
        lines(&[
            "0.5235987755982989 -0 1.5707963267948966 -1.5707963267948966 NaN 1e-200",
            "1.0471975511965979 +0 3.141592653589793 1.5707963267948966 NaN 1.4901161193847656e-8",
            "0.7853981633974483 -0 1.5707963267948966 -1.5707963267948966 1.5707963267948966 -0.4636476090008061",
            "0.7853981633974483 +0 -0 3.141592653589793 -3.141592653589793 1.5707963267948966 -1.5707963267948966 2.356194490192345 -0.7853981633974483 -0 3.141592653589793 NaN 2.498091544796509 0 2",
        ])
    );
}

/// Math.sinh, cosh, tanh, asinh, acosh and atanh. node 24 prints the same lines except 6
/// values, where its libm (fdlibm in V8 13.6) is not correctly rounded: sinh(710),
/// asinh(1e300), acosh(2), acosh(1e300), acosh(1.0000000000000002) and atanh(0.5) end in
/// ...557e+308, ...736, ...166, ...736, ...017e-8 and ...548 there. The lines here are the
/// correctly rounded values (MPFR in the binary64 context gives the same).
#[test]
fn hyperbolic_functions() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.sinh(1), z(Math.sinh(-0)), Math.sinh(710), Math.sinh(-711), Math.sinh(1e-300), Math.sinh(-Infinity)); print(Math.cosh(1), Math.cosh(-0), Math.cosh(711), Math.cosh(-Infinity), Math.cosh(NaN), Math.cosh(1e-8)); print(Math.tanh(1), z(Math.tanh(-0)), Math.tanh(20), Math.tanh(-Infinity), Math.tanh(0.5), Math.tanh(1e-300)); print(Math.asinh(1), z(Math.asinh(-0)), Math.asinh(-Infinity), Math.asinh(1e300), Math.asinh(-1e-10)); print(Math.acosh(2), z(Math.acosh(1)), Math.acosh(0.5), Math.acosh(Infinity), Math.acosh(1e300), Math.acosh(1.0000000000000002)); print(Math.atanh(0.5), z(Math.atanh(-0)), Math.atanh(1), Math.atanh(-1), Math.atanh(2), Math.atanh(-1e-10))"#
        ),
        lines(&[
            "1.1752011936438014 -0 1.1169973830808555e+308 -Infinity 1e-300 -Infinity",
            "1.5430806348152437 1 Infinity Infinity NaN 1",
            "0.7615941559557649 -0 1 -1 0.46211715726000974 1e-300",
            "0.881373587019543 -0 -Infinity 691.4686750787737 -1e-10",
            "1.3169578969248168 +0 NaN Infinity 691.4686750787737 2.1073424255447014e-8",
            "0.5493061443340549 -0 Infinity -Infinity NaN -1e-10",
        ])
    );
}

/// Math.log2, log10, log1p, expm1 and cbrt. node 24 prints the same lines except expm1(1):
/// 1.718281828459045 there (fdlibm); 1.7182818284590453 is the correctly rounded value (MPFR).
#[test]
fn log2_log10_log1p_expm1_cbrt() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.log2(8), z(Math.log2(1)), Math.log2(0), Math.log2(-1), Math.log2(5e-324), Math.log2(3), Math.log2(1.7976931348623157e308)); print(Math.log10(1000), z(Math.log10(1)), Math.log10(-0), Math.log10(2), Math.log10(1e-300), Math.log10(1e23)); print(Math.log1p(1), z(Math.log1p(-0)), Math.log1p(-1), Math.log1p(-2), Math.log1p(1e-20), Math.log1p(Infinity)); print(Math.expm1(1), z(Math.expm1(-0)), Math.expm1(-Infinity), Math.expm1(Infinity), Math.expm1(1e-10), Math.expm1(710), Math.expm1(-40)); print(Math.cbrt(27), z(Math.cbrt(-0)), Math.cbrt(-8), Math.cbrt(2), Math.cbrt(Infinity), Math.cbrt(5e-324), Math.cbrt(-1e300))"#
        ),
        lines(&[
            "3 +0 -Infinity NaN -1074 1.584962500721156 1024",
            "3 +0 -Infinity 0.3010299956639812 -300 23",
            "0.6931471805599453 -0 -Infinity NaN 1e-20 Infinity",
            "1.7182818284590453 -0 -1 Infinity 1.00000000005e-10 Infinity -1",
            "3 -0 -2 1.2599210498948732 Infinity 1.7031839360032603e-108 -1e+100",
        ])
    );
}

/// Math.hypot: V8's MathHypot (the same bits as node); every argument is converted, in
/// order, before the ±∞ and NaN checks, and a throwing valueOf stops the conversions.
#[test]
fn hypot_converts_every_argument_first() {
    assert_eq!(
        common::run_both(
            r#"function z(x) { return x === 0 ? (1 / x > 0 ? '+0' : '-0') : String(x); } print(Math.hypot(3, 4), z(Math.hypot()), z(Math.hypot(-0)), z(Math.hypot(-0, -0)), Math.hypot(-3), Math.hypot(NaN, Infinity), Math.hypot(NaN, 1), Math.hypot(1, 2, 3), Math.hypot(0.1, 0.2, 0.3, 0.4), Math.hypot(1e308, 1e308), Math.hypot(5e-324, 5e-324), Math.hypot('3', '4'), Math.hypot.length); var log = []; function v(n, x) { return { valueOf: function () { log.push(n); return x; } }; } print(Math.hypot(v('a', Infinity), v('b', NaN), v('c', 1)), log.join()); var t = []; try { Math.hypot(v('d', 1), { valueOf: function () { throw new Error('x'); } }, v('e', 2)); } catch (e) { t.push(e.message); } print(t.join(), log.join())"#
        ),
        lines(&[
            "5 +0 +0 +0 3 Infinity NaN 3.741657386773941 0.5477225575051662 1.4142135623730951e+308 5e-324 5 2",
            "Infinity a,b,c",
            "x a,b,c,d",
        ])
    );
}

/// The names, lengths and attributes of the M4-P4 functions; they are not constructors.
#[test]
fn m4_p4_function_lengths_and_names() {
    assert_eq!(
        common::run_both(
            r#"var n = ['acos','acosh','asin','asinh','atan','atanh','atan2','cbrt','cos','cosh','expm1','hypot','log1p','log10','log2','sin','sinh','tan','tanh']; print(n.map(function (k) { return Math[k].name + ':' + Math[k].length; }).join()); var d = Object.getOwnPropertyDescriptor(Math, 'hypot'); print(d.writable, d.enumerable, d.configurable, typeof Math.sin.prototype); try { new Math.sin(1); } catch (e) { print(e.name); }"#
        ),
        lines(&[
            "acos:1,acosh:1,asin:1,asinh:1,atan:1,atanh:1,atan2:2,cbrt:1,cos:1,cosh:1,expm1:1,hypot:2,log1p:1,log10:1,log2:1,sin:1,sinh:1,tan:1,tanh:1",
            "true false true undefined",
            "TypeError",
        ])
    );
}
