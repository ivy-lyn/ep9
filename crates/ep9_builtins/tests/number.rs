//! Number (§21.1): the constructor, the constants, `Number.prototype.toString` (radix
//! 2-36) and `valueOf`.
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
