//! Number (§21.1): the constructor, the constants, `Number.prototype.toString` (radix
//! 2-36), `valueOf`, `toFixed`, `toExponential` and `toPrecision`; the statics isFinite,
//! isInteger, isNaN and isSafeInteger; parseInt and parseFloat (§19.2.4-5), one function
//! object each under `Number` and the global object.
//!
//! The expected lines are node 24's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

use ep9_vm::{Cx, JsResult, Local};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn acceptance_new_number_plus_one() {
    assert_eq!(
        common::run_both(r#"print(new Number(1)+1===2)"#),
        lines(&["true"])
    );
}

#[test]
fn number_called_converts() {
    assert_eq!(
        common::run_both(
            r#"print(Number(), Number(undefined), Number('12'), Number(' 0x10 '), Number(true), Number(null), Number(''), Number('1e3'), Number([5]))"#
        ),
        lines(&["0 NaN 12 16 1 0 0 1000 5"])
    );
}

#[test]
fn number_called_returns_a_primitive() {
    assert_eq!(
        common::run_both(
            r#"print(typeof Number(1), typeof new Number(1), new Number(7) instanceof Number, new Number(7).constructor === Number)"#
        ),
        lines(&["number object true true"])
    );
}

#[test]
fn number_converts_once_with_valueof() {
    assert_eq!(
        common::run_both(
            r#"var n = 0; var v = Number({ valueOf: function () { n++; return 4; } }); print(v, n)"#
        ),
        lines(&["4 1"])
    );
}

#[test]
fn number_wrapper_unwraps() {
    assert_eq!(
        common::run_both(
            r#"var w = new Number(2.5); print(w + 1, w * 2, w.valueOf(), w.toString(), w == 2.5, w === 2.5)"#
        ),
        lines(&["3.5 5 2.5 2.5 true false"])
    );
}

#[test]
fn number_subclass_gets_new_target_prototype() {
    assert_eq!(
        common::run_both(
            r#"class N extends Number { constructor(x) { super(x); } } var n = new N(3); print(n instanceof N, n instanceof Number, n + 1, n.toString(2), n.constructor === N)"#
        ),
        lines(&["true true 4 11 true"])
    );
}

#[test]
fn number_constants() {
    assert_eq!(
        common::run_both(
            r#"print(Number.MAX_VALUE, Number.MIN_VALUE, Number.NaN, Number.NEGATIVE_INFINITY, Number.POSITIVE_INFINITY, Number.MAX_SAFE_INTEGER, Number.MIN_SAFE_INTEGER, Number.EPSILON, Number.EPSILON === 2.220446049250313e-16)"#
        ),
        lines(&[
            "1.7976931348623157e+308 5e-324 NaN -Infinity Infinity 9007199254740991 -9007199254740991 2.220446049250313e-16 true"
        ])
    );
}

#[test]
fn number_constants_are_read_only() {
    assert_eq!(
        common::run_both(
            r#"Number.MAX_VALUE = 1; delete Number.NaN; var keys = ''; for (var k in Number) keys += k; print(Number.MAX_VALUE, Number.NaN, delete Number.EPSILON, keys === '')"#
        ),
        lines(&["1.7976931348623157e+308 NaN false true"])
    );
}

#[test]
fn number_constants_throw_in_strict_code() {
    assert_eq!(
        common::run_both(
            r#"'use strict'; try { Number.EPSILON = 1; print('no throw'); } catch (e) { print(e.name); }"#
        ),
        lines(&["TypeError"])
    );
}

#[test]
fn number_functions_have_length_and_name() {
    assert_eq!(
        common::run_both(
            r#"print(Number.length, Number.name, Number.prototype.toString.length, Number.prototype.toString.name, Number.prototype.valueOf.length, Number.prototype.valueOf.name)"#
        ),
        lines(&["1 Number 1 toString 0 valueOf"])
    );
}

#[test]
fn number_prototype_is_a_wrapper_of_zero() {
    assert_eq!(
        common::run_both(
            r#"print(Number.prototype.valueOf(), Number.prototype.toString(), Number.prototype.constructor === Number, typeof Number.prototype)"#
        ),
        lines(&["0 0 true object"])
    );
}

#[test]
fn to_string_radix() {
    assert_eq!(
        common::run_both(
            r#"print((255).toString(16), (255).toString(), (255).toString(undefined), (-255.5).toString(2), (0.5).toString(36), (1e21).toString(36), (35).toString(36), (1).toString(10.9), (3).toString(2.5))"#
        ),
        lines(&["ff 255 255 -11111111.1 0.i 5v1j4f4ds7c000 z 1 11"])
    );
}

#[test]
fn to_string_radix_special_values() {
    assert_eq!(
        common::run_both(
            r#"print(NaN.toString(2), (-Infinity).toString(36), Infinity.toString(16), (-0).toString(2), (0.1).toString(3), (-1.5).toString(7))"#
        ),
        lines(&[
            "NaN -Infinity Infinity 0 0.0022002200220022002200220022002201 -1.3333333333333333334"
        ])
    );
}

#[test]
fn to_string_radix_out_of_range_is_a_range_error() {
    assert_eq!(
        common::run_both(
            r#"var r = []; for (var x of [1, 37, 0, Infinity]) { try { (1).toString(x); r.push('ok'); } catch (e) { r.push(e.name); } } print(r.join())"#
        ),
        lines(&["RangeError,RangeError,RangeError,RangeError"])
    );
}

#[test]
fn to_string_and_value_of_check_this() {
    assert_eq!(
        common::run_both(
            r#"function callOn(t, f, a) { Object.prototype.__f = f; try { return t.__f(a); } finally { delete Object.prototype.__f; } } var r = []; for (var t of ['1', {}, new String('1'), true, 2, new Number(3)]) { try { r.push(callOn(t, Number.prototype.valueOf)); } catch (e) { r.push(e.name); } try { r.push(callOn(t, Number.prototype.toString)); } catch (e) { r.push(e.name); } } var f = Number.prototype.valueOf; try { f(); } catch (e) { r.push(e.name); } print(r.join())"#
        ),
        lines(&[
            "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,2,2,3,3,TypeError"
        ])
    );
}

#[test]
fn this_number_value_reads_this_before_radix() {
    assert_eq!(
        common::run_both(
            r#"function callOn(t, f, a) { Object.prototype.__f = f; try { return t.__f(a); } finally { delete Object.prototype.__f; } } var log = []; try { callOn('x', Number.prototype.toString, { valueOf: function () { log.push('radix'); return 2; } }); } catch (e) { log.push(e.name); } print(log.join())"#
        ),
        lines(&["TypeError"])
    );
}

/// The identity `Number.parseInt === parseInt` (§21.1.2.13) and `Number.parseFloat ===
/// parseFloat` (§21.1.2.12), and the four predicates, which do not convert.
#[test]
fn number_statics() {
    assert_eq!(
        common::run_both(
            r#"
            print(Number.parseInt === parseInt, Number.parseFloat === parseFloat, parseInt.name, parseInt.length, parseFloat.name, parseFloat.length);
            print(Number.isFinite.length, Number.isInteger.name, Number.isNaN.length, Number.isSafeInteger.length);
            print(Number.isFinite(1), Number.isFinite('1'), Number.isFinite(Infinity), Number.isFinite(NaN), Number.isFinite(new Number(1)));
            print(Number.isInteger(5), Number.isInteger(5.5), Number.isInteger(-0), Number.isInteger('5'), Number.isInteger(Infinity), Number.isInteger(1e300));
            print(Number.isNaN(NaN), Number.isNaN('x'), Number.isNaN(undefined), Number.isNaN(0 / 0));
            print(Number.isSafeInteger(9007199254740991), Number.isSafeInteger(9007199254740992), Number.isSafeInteger(-9007199254740991), Number.isSafeInteger(1.5), Number.isSafeInteger('1'));
"#
        ),
        lines(&[
            "true true parseInt 2 parseFloat 1",
            "1 isInteger 1 1",
            "true false false false false",
            "true false true false false true",
            "true false false true",
            "true false true false false",
        ])
    );
}

/// parseInt (§19.2.5): whitespace, sign, the 0x prefix with radix 0 and 16, the radix range
/// after ToInt32, empty digits, long decimal strings (correctly rounded), radix 2 and 16
/// rounding (half to even past 2^53), the generic radices, and ToString before ToInt32.
#[test]
fn parse_int() {
    assert_eq!(
        common::run_both(
            r#"
            function show(x) { return (x === 0 && 1 / x < 0) ? '-0' : String(x); }
            print([parseInt('  42px'), parseInt('-0'), parseInt('0x1F'), parseInt('0x1F', 16), parseInt('0x1F', 10), parseInt('1F', 16), parseInt('z', 36), parseInt('Z', 36)].map(show).join());
            print([parseInt(''), parseInt('x'), parseInt('12', 1), parseInt('12', 37), parseInt('12', -1), parseInt('12', 0), parseInt('12', 4294967306), parseInt('11', 2.9), parseInt('0x'), parseInt('-0x10')].map(show).join());
            print([parseInt('\u00a0\ufeff\u2028\u3000\t\n 7'), parseInt('\u180e7'), parseInt('\u200b7'), parseInt('+-1'), parseInt('\u0663'), parseInt('1e3'), parseInt('0.0000001'), parseInt(0.0000001), parseInt(1e21)].map(show).join());
            print([parseInt('9007199254740993'), parseInt('123456789012345678901234567890'), parseInt('1'.repeat(400)), parseInt('0'.repeat(500) + '7')].map(show).join());
            print([parseInt('1'.repeat(54), 2), parseInt('10000000000000000000000000000000000000000000000000000011', 2), parseInt('100000000000000000000000000000000000000000000000000001', 2), parseInt('fffffffffffffc00', 16), parseInt('fffffffffffffbff', 16), parseInt('20000000000001', 16)].map(show).join());
            print([parseInt('zzzzzzzzzzzzz', 36), parseInt('2222222222222222222222222222222222', 3), parseInt('-666666666666666666666', 7)].map(show).join());
            var order = []; parseInt({ toString: function () { order.push('s'); return '10'; } }, { valueOf: function () { order.push('r'); return 16; } }); print(order.join());
"#
        ),
        lines(&[
            "42,-0,31,31,0,31,35,35",
            "NaN,NaN,NaN,NaN,NaN,12,12,3,NaN,-16",
            "7,NaN,NaN,NaN,NaN,1,0,1,1",
            "9007199254740992,1.2345678901234568e+29,Infinity,7",
            "18014398509481984,36028797018963970,9007199254740992,18446744073709552000,18446744073709550000,9007199254740992",
            "170581728179578200000,16677181699666568,-558545864083284030",
            "s,r",
        ])
    );
}

/// parseFloat (§19.2.4): the longest StrDecimalLiteral prefix, Infinity by hand (case
/// sensitive), no hex or separators, overflow and underflow, correct rounding, conversions.
#[test]
fn parse_float() {
    assert_eq!(
        common::run_both(
            r#"
            function show(x) { return (x === 0 && 1 / x < 0) ? '-0' : String(x); }
            print([parseFloat('3.14abc'), parseFloat('  -.5'), parseFloat('.'), parseFloat('-.e1'), parseFloat('1.'), parseFloat('1.e2'), parseFloat('1e'), parseFloat('1e+'), parseFloat('1e-2x'), parseFloat('+1E+2')].map(show).join());
            print([parseFloat('Infinity'), parseFloat('-Infinityx'), parseFloat('+Infinity'), parseFloat('infinity'), parseFloat('Inf'), parseFloat('NaN'), parseFloat('-0'), parseFloat('0x10'), parseFloat('1_000'), parseFloat('')].map(show).join());
            print([parseFloat('\u00a0\ufeff 2.5'), parseFloat('\u180e2.5'), parseFloat('1e400'), parseFloat('-1e400'), parseFloat('1e-400'), parseFloat('2.4703282292062328e-324'), parseFloat('2.4703282292062327e-324'), parseFloat('9007199254740993'), parseFloat('0.1e1'), parseFloat('00012')].map(show).join());
            print([parseFloat(1e21), parseFloat(-0), parseFloat({ toString: function () { return '7.5'; } }), parseFloat([' 8']), parseFloat(null), parseFloat(true)].map(show).join());
            try { parseFloat(Symbol()); } catch (e) { print(e.name); }
            try { parseInt('1', Symbol()); } catch (e) { print(e.name); }
"#
        ),
        lines(&[
            "3.14,-0.5,NaN,NaN,1,100,1,1,0.01,100",
            "Infinity,-Infinity,Infinity,NaN,NaN,NaN,-0,0,1,NaN",
            "2.5,NaN,Infinity,-Infinity,0,5e-324,0,9007199254740992,1,12",
            "1e+21,0,7.5,8,NaN,NaN",
            "TypeError",
            "TypeError",
        ])
    );
}

/// The keys of `%Number%` and the global's Number, parseFloat, parseInt in V8's order (node 24
/// `Object.getOwnPropertyNames`; ep9 puts Array before Function, so those two are left out),
/// the {w, c} attributes, and no `[[Construct]]`.
#[test]
fn key_order_and_attributes() {
    assert_eq!(
        common::run_both(
            r#"
            print(Object.getOwnPropertyNames(Number).join());
            var g = Object.getOwnPropertyNames(globalThis), want = ['Object', 'Number', 'parseFloat', 'parseInt', 'Boolean', 'String', 'Symbol'];
            print(g.filter(function (k) { return want.indexOf(k) >= 0; }).join());
            var d = Object.getOwnPropertyDescriptor(globalThis, 'parseInt'), e = Object.getOwnPropertyDescriptor(Number, 'parseFloat'), f = Object.getOwnPropertyDescriptor(Number, 'isNaN');
            print(d.writable, d.enumerable, d.configurable, e.writable, e.enumerable, e.configurable, f.writable, f.enumerable, f.configurable);
            try { new parseInt('1'); } catch (err) { print(err.name); }
            try { new Number.isNaN(1); } catch (err) { print(err.name); }
"#
        ),
        lines(&[
            "length,name,prototype,isFinite,isInteger,isNaN,isSafeInteger,parseFloat,parseInt,MAX_VALUE,MIN_VALUE,NaN,NEGATIVE_INFINITY,POSITIVE_INFINITY,MAX_SAFE_INTEGER,MIN_SAFE_INTEGER,EPSILON",
            "Object,Number,parseFloat,parseInt,Boolean,String,Symbol",
            "true false true true false true true false true",
            "TypeError",
            "TypeError",
        ])
    );
}

/// StrWhiteSpaceChar: this module's copy of the table (`number::is_str_white_space`,
/// M2-P5-PLAN-D5) equals the code units that String.prototype.trim removes, on all 65,536 code
/// units, and parseInt and parseFloat skip exactly those (U+180E and U+200B are not in it).
/// Without GC stress: the loop makes about 400,000 strings, and a collection before each one
/// takes minutes in a debug build; the other tests here cover these functions under stress.
#[test]
fn white_space_table_matches_trim() {
    let got = common::on_big_stack(|| {
        common::run(
            r#"
        var out = [], bad = [];
        for (var u = 0; u < 65536; u++) {
          var c = String.fromCharCode(u);
          if ((c + 'x' + c).trim() === 'x') out.push(u);
          var trimmed = (c + '1').trim() === '1';
          var skipped = parseInt(c + '1') === 1 && parseFloat(c + '1') === 1 && c !== '+' && c !== '0';
          if (trimmed !== skipped) bad.push(u);
        }
        print(out.join());
        print(bad.length);
"#,
            false,
        )
    })
    .expect("no exception");
    let table: Vec<String> = (0..=u16::MAX)
        .filter(|&u| ep9_builtins::number::is_str_white_space(u))
        .map(|u| u.to_string())
        .collect();
    assert_eq!(got, vec![table.join(","), "0".to_string()]);
    assert!(!ep9_builtins::number::is_str_white_space(0x180E));
}

// ---- toFixed, toExponential, toPrecision (§21.1.3.2, 3, 5) --------------------------------------

/// The argument checks in spec order: thisNumberValue first, then ToIntegerOrInfinity, then
/// the non-finite answer (toFixed: after its range check; toExponential and toPrecision:
/// before it), then the RangeError.
#[test]
fn digit_methods_check_arguments_in_spec_order() {
    assert_eq!(
        common::run_both(
            r#"
            function r(f) { try { return String(f()); } catch (e) { return e.name; } }
            print([r(() => (1).toFixed(101)), r(() => (1).toFixed(-1)), r(() => (1).toFixed(Infinity)), r(() => NaN.toFixed(101)), r(() => NaN.toFixed(Infinity)), r(() => (1).toFixed(100.9)), r(() => (1).toFixed(-0.9))].join());
            print([r(() => (1).toExponential(101)), r(() => (1).toExponential(-1)), r(() => NaN.toExponential(Infinity)), r(() => Infinity.toExponential(-1)), r(() => (-Infinity).toExponential(1000))].join());
            print([r(() => (1).toPrecision(0)), r(() => (1).toPrecision(101)), r(() => NaN.toPrecision(0)), r(() => Infinity.toPrecision(101)), r(() => (1).toPrecision(undefined)), r(() => (1.5).toPrecision())].join());
            var log = [];
            var p = { valueOf: function () { log.push('p'); return 2; } };
            print((1.25).toFixed(p), (1.25).toExponential(p), (1.25).toPrecision(p), log.join(''));
            var bad = { valueOf: function () { log.push('bad'); return 2; } };
            print(r(() => Number.prototype.toFixed.call('1', bad)), r(() => Number.prototype.toExponential.call({}, bad)), r(() => Number.prototype.toPrecision.call(null, bad)), log.join(''));
            print((new Number(1.5)).toFixed(2), Number.prototype.toFixed.call(new Number(-2.5)), r(() => (1).toFixed(Symbol())));
"#
        ),
        lines(&[
            "RangeError,RangeError,RangeError,RangeError,RangeError,1.0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000,1",
            "RangeError,RangeError,NaN,Infinity,-Infinity",
            "RangeError,RangeError,NaN,Infinity,1,1.5",
            "1.25 1.25e+0 1.3 ppp",
            "TypeError TypeError TypeError ppp",
            "1.50 -3 TypeError",
        ])
    );
}

/// Values node prints (node 24.22.1): exact halves round up ("the larger n"), values that only
/// look like halves follow their exact binary value (1.005 is 1.00499…; 1.5e-7 is 1.49…e-7),
/// -0 has no sign, toFixed of 10^21 and above is ToString, and the exponent forms.
#[test]
fn digit_methods_match_node() {
    assert_eq!(
        common::run_both(
            r#"
            print([(1.005).toFixed(2), (1.255).toFixed(2), (2.5).toFixed(0), (-2.5).toFixed(0), (0.5).toFixed(0), (1.45).toFixed(1), (8.345).toFixed(2), (0.000001).toFixed(7), (123.456).toFixed(10)].join());
            print([(-0).toFixed(2), (-1e-7).toFixed(2), (0).toFixed(), (1e21).toFixed(2), (-1e21).toFixed(), (999999999999999900000).toFixed(1), (0.1).toFixed(20), (9.995).toFixed(2), (99.995).toFixed(2)].join());
            print([(5e-324).toFixed(100).length, (1.7976931348623157e308).toFixed(3), (0.00001).toFixed(5), (0.000005).toFixed(5)].join());
            print([(123.456).toExponential(), (0).toExponential(), (-0).toExponential(2), (1e21).toExponential(), (5e-324).toExponential(), (1.7976931348623157e308).toExponential(5), (1.25).toExponential(1), (1.35).toExponential(1), (9.99).toExponential(1), (-123).toExponential(0)].join());
            print([(123.456).toPrecision(4), (0.000001234).toPrecision(2), (0.0000001234).toPrecision(2), (123456).toPrecision(2), (123456).toPrecision(6), (99.99).toPrecision(3), (0).toPrecision(5), (-0).toPrecision(1), (1e21).toPrecision(3), (2.5).toPrecision(1), (1.5e-7).toPrecision(1)].join());
            print([(900719925474099.25).toExponential(), (900719925474099.25).toPrecision(16), (900719925474099.25).toFixed(1), (0.1).toPrecision(21), (5e-324).toPrecision(3)].join());
"#
        ),
        lines(&[
            "1.00,1.25,3,-3,1,1.4,8.35,0.0000010,123.4560000000",
            "0.00,-0.00,0,1e+21,-1e+21,999999999999999868928.0,0.10000000000000000555,9.99,100.00",
            "102,1.7976931348623157e+308,0.00001,0.00001",
            "1.23456e+2,0e+0,0.00e+0,1e+21,5e-324,1.79769e+308,1.3e+0,1.4e+0,1.0e+1,-1e+2",
            "123.5,0.0000012,1.2e-7,1.2e+5,123456,100,0.0000,0,1.00e+21,3,1e-7",
            "9.007199254740992e+14,900719925474099.3,900719925474099.3,0.100000000000000005551,4.94e-324",
        ])
    );
}

/// length, name, the keys of `%Number.prototype%` in V8's order (toLocaleString is not built),
/// {w, c}, no [[Construct]].
#[test]
fn digit_methods_have_length_name_and_keys() {
    assert_eq!(
        common::run_both(
            r#"
            var P = Number.prototype;
            print(P.toFixed.length, P.toFixed.name, P.toExponential.length, P.toExponential.name, P.toPrecision.length, P.toPrecision.name);
            print(Object.getOwnPropertyNames(P).filter(k => k !== 'toLocaleString').join());
            var d = Object.getOwnPropertyDescriptor(P, 'toPrecision');
            print(d.writable, d.enumerable, d.configurable);
            try { new P.toFixed(1); } catch (e) { print(e.name); }
"#
        ),
        lines(&[
            "1 toFixed 1 toExponential 1 toPrecision",
            "constructor,toExponential,toFixed,toPrecision,toString,valueOf",
            "true false true",
            "TypeError",
        ])
    );
}

/// The exact digits and the half-up rounding (`ep9_builtins::number` helpers).
#[test]
fn exact_decimal_and_rounding() {
    use ep9_builtins::number::{exact_decimal, round_half_up};
    let digits = |x: f64| {
        let (d, p) = exact_decimal(x);
        (
            d.iter().map(|c| char::from(b'0' + c)).collect::<String>(),
            p,
        )
    };
    assert_eq!(digits(1.0), ("1".into(), 1));
    assert_eq!(digits(0.5), ("5".into(), 0));
    assert_eq!(digits(1234.0), ("1234".into(), 4));
    assert_eq!(digits(0.125), ("125".into(), 0));
    assert_eq!(digits(1e21), ("1".into(), 22));
    // 1e-7 is 9.99…e-8.
    assert_eq!(digits(1e-7).1, -7);
    assert_eq!(
        digits(0.1).0,
        "1000000000000000055511151231257827021181583404541015625"
    );
    // 5e-324 = 2^-1074: 751 significant digits, the first at 10^-324.
    let (d, p) = digits(5e-324);
    assert_eq!((d.len(), p, &d[..6]), (751, -323, "494065"));
    // 2^1024 - 2^971: 309 digits.
    assert_eq!(digits(f64::MAX).0.len(), 309);
    assert_eq!(digits(f64::MAX).1, 309);
    // Rounding: carry through nines, a cut above the first digit, padding.
    assert_eq!(round_half_up(&[9, 9, 5], 1, 2), (vec![1, 0], 2));
    assert_eq!(round_half_up(&[9, 9, 4], 1, 2), (vec![9, 9], 1));
    assert_eq!(round_half_up(&[5], -2, 0), (vec![1], -1));
    assert_eq!(round_half_up(&[4, 9], -2, 0), (vec![], -2));
    assert_eq!(round_half_up(&[9], 0, -1), (vec![], 0));
    assert_eq!(round_half_up(&[1, 2], 3, 4), (vec![1, 2, 0, 0], 3));
}

/// The node script of the golden below (FNV-1a 32 of each block's strings, each followed by
/// LF). Regenerate: `cargo test -p ep9_builtins --test number -- --ignored node_golden`
/// prints the table when node differs. Doubles `10b .. 10b + 9` make block `b`: kind `i % 5`
/// of double `i` is 0 any bits (NaN, infinities and huge or tiny values too), 1 a random
/// value in [2^-40, 2^70), 2 an integer below 2^20 over 2^0-2^15 (exact ties at many digit
/// counts), 3 an integer below 2^24 over 10^0-10^9 (values like 1.005), 4 one of the 17
/// doubles around 10^0-10^22 (the 10^21 edge of toFixed).
const GOLDEN_SCRIPT: &str = r##"// The node golden of toFixed, toExponential and toPrecision (ep9_builtins tests/number.rs).
// node -e SCRIPT                 -> "block <b> <fnv1a32>" for the 100 blocks of 10 doubles
// GOLDEN_BLOCK=b node -e SCRIPT  -> the 3,040 strings of block b, one per line
const MASK = (1n << 64n) - 1n;
let state = 0x9e3779b97f4a7c15n;
function next() {
  state ^= (state << 13n) & MASK;
  state ^= state >> 7n;
  state ^= (state << 17n) & MASK;
  return state;
}
const dv = new DataView(new ArrayBuffer(8));
const fromBits = (b) => { dv.setBigUint64(0, b); return dv.getFloat64(0); };
const toBits = (x) => { dv.setFloat64(0, x); return dv.getBigUint64(0); };
const P10 = Array.from({ length: 23 }, (_, j) => Number('1e' + j));
function goldenDouble(i) {
  const r = next(), t = next();
  let x;
  switch (i % 5) {
    case 0: return fromBits(r);
    case 1: return fromBits((r & (1n << 63n)) | ((983n + t % 110n) << 52n) | (r & ((1n << 52n) - 1n)));
    case 2: x = Number(r >> 44n) / 2 ** Number(t % 16n); break;
    case 3: x = Number(r >> 40n) / P10[Number(t % 10n)]; break;
    case 4: x = fromBits(toBits(P10[Number(t % 23n)]) + r % 17n - 8n); break;
  }
  return t >> 63n ? -x : x;
}
function strings(x) {
  const out = [];
  for (let f = 0; f <= 100; f++) out.push(x.toFixed(f));
  out.push(x.toExponential());
  for (let f = 0; f <= 100; f++) out.push(x.toExponential(f));
  out.push(x.toPrecision());
  for (let p = 1; p <= 100; p++) out.push(x.toPrecision(p));
  return out;
}
function fnv(list) {
  let h = 0x811c9dc5;
  for (const s of list) {
    for (const c of s + '\n') { h ^= c.charCodeAt(0); h = Math.imul(h, 0x01000193) >>> 0; }
  }
  return h.toString(16).padStart(8, '0');
}
const want = process.env.GOLDEN_BLOCK === undefined ? -1 : Number(process.env.GOLDEN_BLOCK);
for (let b = 0; b < 100; b++) {
  const list = [];
  for (let i = 0; i < 10; i++) list.push(...strings(goldenDouble(b * 10 + i)));
  if (want < 0) console.log(`block ${b} ${fnv(list)}`);
  else if (b === want) console.log(list.join('\n'));
}
"##;

/// `node -e GOLDEN_SCRIPT` with node 24.22.1.
const GOLDEN: &str = "block 0 0cbfb046
block 1 5bf86ed2
block 2 5a1abfc5
block 3 b6c1e3f4
block 4 57632767
block 5 ed88d8cb
block 6 70ec3532
block 7 c7904a55
block 8 3ba72cae
block 9 e4aaf7d8
block 10 f034006f
block 11 2032b801
block 12 e0487e79
block 13 981599cb
block 14 6412f421
block 15 fc3f2854
block 16 6e06c4f6
block 17 b34e2f16
block 18 c67659b2
block 19 392f8377
block 20 6846ca11
block 21 6bed7678
block 22 1722eeee
block 23 747d42cf
block 24 ac629ee8
block 25 600f921b
block 26 8e47287d
block 27 9605771e
block 28 0e90da03
block 29 857c6a83
block 30 919b52ec
block 31 cce70f2d
block 32 738cb416
block 33 d98cb764
block 34 a86fd462
block 35 40cbe74f
block 36 ca70efe1
block 37 1bb54b7d
block 38 e3d7c2c9
block 39 4f2940bf
block 40 a2c2a59f
block 41 f6644bff
block 42 59560995
block 43 8afbbb26
block 44 6d8780dc
block 45 05ee2ec4
block 46 aa3fb3c8
block 47 02c35248
block 48 6e670489
block 49 d3d8f3d6
block 50 5962a23c
block 51 f250b176
block 52 a421bf8c
block 53 26ee452d
block 54 935e9b41
block 55 f46ac824
block 56 55d9b594
block 57 744663fb
block 58 7bb2253d
block 59 9422e7af
block 60 bcea752c
block 61 063e7cf6
block 62 0dfedccf
block 63 840a32ec
block 64 7df32d56
block 65 a3b9d490
block 66 c1d18b6c
block 67 0eb2ea0d
block 68 eda0c4e8
block 69 59af1d76
block 70 cfe582ec
block 71 50fc5e1a
block 72 a3a96efc
block 73 0b235e16
block 74 1c07d5f4
block 75 cbe9fc1a
block 76 3da7ac60
block 77 de535d3e
block 78 98878037
block 79 d207c073
block 80 2a1400ab
block 81 29118f97
block 82 2343dbf3
block 83 c1424010
block 84 b0026b29
block 85 564b1f92
block 86 998fe7db
block 87 44297f3a
block 88 49f30c1e
block 89 4edf697c
block 90 4e33defb
block 91 6fb9ad24
block 92 57b23e80
block 93 83537f78
block 94 26eb7338
block 95 57a15f39
block 96 ea205cf1
block 97 831711ec
block 98 8f31fe02
block 99 7bfca7e1
";

/// xorshift64, the generator of the golden script.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// The exact doubles 10^0 .. 10^22 (decimal literals; clippy.toml bans `powi`, H36).
const P10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];

/// Double `i` of the golden (the script's `goldenDouble`).
fn golden_double(g: &mut XorShift, i: u64) -> f64 {
    let r = g.next();
    let t = g.next();
    let x = match i % 5 {
        0 => return f64::from_bits(r),
        1 => {
            let biased = 983 + t % 110;
            return f64::from_bits((r & (1 << 63)) | (biased << 52) | (r & ((1 << 52) - 1)));
        }
        2 => (r >> 44) as f64 / f64::from_bits((1023 + t % 16) << 52),
        3 => (r >> 40) as f64 / P10[(t % 10) as usize],
        _ => f64::from_bits((P10[(t % 23) as usize].to_bits() + r % 17) - 8),
    };
    if t >> 63 == 1 { -x } else { x }
}

/// The 304 strings of one double: toFixed(0..=100), toExponential(), toExponential(0..=100),
/// toPrecision(), toPrecision(1..=100), through `Cx::call` of the installed functions.
fn digit_strings(cx: &mut Cx<'_>, x: f64) -> JsResult<Vec<String>> {
    let proto = cx.intrinsic(ep9_vm::IntrinsicId::NumberPrototype);
    let mut out = Vec::with_capacity(304);
    for (name, args) in [
        ("toFixed", (0..=100).map(Some).collect::<Vec<_>>()),
        (
            "toExponential",
            core::iter::once(None).chain((0..=100).map(Some)).collect(),
        ),
        (
            "toPrecision",
            core::iter::once(None).chain((1..=100).map(Some)).collect(),
        ),
    ] {
        let f = cx.get(proto, name)?;
        for a in args {
            let arg = match a {
                Some(n) => Local::number(f64::from(n)),
                None => Local::undefined(),
            };
            let s = cx.call(f, Local::number(x), &[arg])?;
            out.push(cx.to_rust_string(s)?);
        }
    }
    Ok(out)
}

/// FNV-1a 32 of the strings, each followed by LF.
fn fnv(strings: &[String]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for s in strings {
        for b in s.bytes().chain(b"\n".iter().copied()) {
            h ^= u32::from(b);
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    h
}

/// node's strings of block `b`, if node runs here.
fn node_block(b: usize) -> Option<Vec<String>> {
    let out = std::process::Command::new("node")
        .arg("-e")
        .arg(GOLDEN_SCRIPT)
        .env("GOLDEN_BLOCK", b.to_string())
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.lines().map(str::to_string).collect())
}

/// toFixed, toExponential and toPrecision of 1,000 doubles, every digit count (304 strings
/// each), equal node 24.22.1's: 100 blocks of 10 doubles, each pinned by a hash.
#[test]
fn digit_methods_match_the_node_golden() {
    let mut iso = common::isolate(false);
    let mut g = XorShift(0x9e37_79b9_7f4a_7c15);
    let mut bad = Vec::new();
    for (b, line) in GOLDEN.lines().enumerate() {
        let strings: Vec<String> = iso
            .with_cx(|cx| {
                let mut v = Vec::new();
                for i in 0..10 {
                    let x = golden_double(&mut g, (b * 10 + i) as u64);
                    v.extend(digit_strings(cx, x)?);
                }
                Ok(v)
            })
            .expect("no exception");
        let got = format!("block {b} {:08x}", fnv(&strings));
        if got != line {
            // Name the first differing string when node is here.
            let first = node_block(b).and_then(|node| {
                strings
                    .iter()
                    .zip(&node)
                    .position(|(a, n)| a != n)
                    .map(|k| format!("string {k}: ep9 {:?}, node {:?}", strings[k], node[k]))
            });
            bad.push(format!(
                "{got} (node: {line}); {}",
                first.unwrap_or_default()
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} blocks differ:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// The golden table equals what node prints now (needs node 24.22.1 on PATH).
#[test]
#[ignore = "runs node"]
fn node_golden_is_current() {
    let out = std::process::Command::new("node")
        .arg("-e")
        .arg(GOLDEN_SCRIPT)
        .output()
        .expect("node on PATH");
    let text = String::from_utf8(out.stdout).expect("UTF-8");
    assert_eq!(text, GOLDEN, "the new table:\n{text}");
}
