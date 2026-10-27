//! Number (§21.1): the constructor, the constants, `Number.prototype.toString` (radix
//! 2-36) and `valueOf`; the statics isFinite, isInteger, isNaN and isSafeInteger; parseInt
//! and parseFloat (§19.2.4-5), one function object each under `Number` and the global object.
//!
//! The expected lines are node 24's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

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
