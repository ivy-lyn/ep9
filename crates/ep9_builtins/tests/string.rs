//! String (§22.1): `String(v)`, `new String`, `String.fromCharCode`, and
//! `String.prototype.charCodeAt`, `toString` and `valueOf`.
//!
//! The expected lines are node 24's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn acceptance_object_ab_length_and_typeof_new_string() {
    assert_eq!(
        common::run_both(r#"print(Object('ab').length===2, typeof new String('')==='object')"#),
        lines(&["true true"])
    );
}

#[test]
fn string_called_converts() {
    assert_eq!(
        common::run_both(
            r#"print(String(), String(undefined), String(null), String(1.5), String(-0), String(true), String([1, 2]), String({}), String('x'), String(1e21))"#
        ),
        lines(&[" undefined null 1.5 0 true 1,2 [object Object] x 1e+21"])
    );
}

#[test]
fn string_called_on_a_symbol_gives_its_descriptive_string() {
    assert_eq!(
        common::run_both(
            r#"print(String(Symbol('d')), String(Symbol()), String(Symbol.iterator), String(Symbol.for('r')), String(Symbol('')))"#
        ),
        lines(&["Symbol(d) Symbol() Symbol(Symbol.iterator) Symbol(r) Symbol()"])
    );
}

#[test]
fn new_string_on_a_symbol_is_a_type_error() {
    assert_eq!(
        common::run_both(
            r#"try { new String(Symbol('d')); print('no throw'); } catch (e) { print(e.name); } try { '' + Symbol(); } catch (e) { print(e.name); }"#
        ),
        lines(&["TypeError", "TypeError"])
    );
}

#[test]
fn new_string_is_a_string_exotic_object() {
    assert_eq!(
        common::run_both(
            r#"var s = new String('abc'); print(typeof s, s.length, s[0], s[2], s[3], s + '!', s.toString(), s.valueOf(), s instanceof String, s == 'abc', s === 'abc')"#
        ),
        lines(&["object 3 a c undefined abc! abc abc true true false"])
    );
}

#[test]
fn string_subclass() {
    assert_eq!(
        common::run_both(
            r#"class S extends String {} var s = new S('hey'); print(s instanceof S, s instanceof String, s.length, s[1], s + '!', s.charCodeAt(0))"#
        ),
        lines(&["true true 3 e hey! 104"])
    );
}

#[test]
fn from_char_code() {
    assert_eq!(
        common::run_both(
            r#"print(String.fromCharCode(), String.fromCharCode(65, 66, 67), String.fromCharCode(65.9, '66'), String.fromCharCode(0x10041) === 'A', String.fromCharCode(-1).charCodeAt(0), String.fromCharCode(0xD83D, 0xDE00).length, String.fromCharCode(NaN).charCodeAt(0))"#
        ),
        lines(&[" ABC AB true 65535 2 0"])
    );
}

#[test]
fn from_char_code_converts_in_order() {
    assert_eq!(
        common::run_both(
            r#"var log = []; String.fromCharCode({ valueOf: function () { log.push('a'); return 65; } }, { valueOf: function () { log.push('b'); return 66; } }); print(log.join())"#
        ),
        lines(&["a,b"])
    );
}

#[test]
fn char_code_at() {
    assert_eq!(
        common::run_both(
            r#"print('abc'.charCodeAt(0), 'abc'.charCodeAt(2), 'abc'.charCodeAt(3), 'abc'.charCodeAt(-1), 'abc'.charCodeAt(), 'abc'.charCodeAt(NaN), 'abc'.charCodeAt('1'), 'abc'.charCodeAt(1.9), 'abc'.charCodeAt(Infinity), '\uD83D\uDE00'.charCodeAt(1), ''.charCodeAt(0))"#
        ),
        lines(&["97 99 NaN NaN 97 97 98 98 NaN 56832 NaN"])
    );
}

#[test]
fn char_code_at_on_other_values() {
    assert_eq!(
        common::run_both(
            r#"function callOn(t, f, a) { Object.prototype.__f = f; try { return t.__f(a); } finally { delete Object.prototype.__f; } } print(callOn(123, String.prototype.charCodeAt, 1), callOn(true, String.prototype.charCodeAt, 0), new String('xy').charCodeAt(1), callOn({}, String.prototype.charCodeAt, 1)); var f = String.prototype.charCodeAt; try { f(0); } catch (e) { print(e.name); }"#
        ),
        lines(&["50 116 121 111", "TypeError"])
    );
}

#[test]
fn char_code_at_converts_this_before_pos() {
    assert_eq!(
        common::run_both(
            r#"var log = []; var o = { toString: function () { log.push('this'); return 'ab'; }, cc: String.prototype.charCodeAt }; var r = o.cc({ valueOf: function () { log.push('pos'); return 1; } }); print(r, log.join())"#
        ),
        lines(&["98 this,pos"])
    );
}

#[test]
fn string_to_string_and_value_of_check_this() {
    assert_eq!(
        common::run_both(
            r#"function callOn(t, f, a) { Object.prototype.__f = f; try { return t.__f(a); } finally { delete Object.prototype.__f; } } var r = []; for (var t of [1, {}, new Number(1), Symbol(), 'v', new String('w')]) { try { r.push(callOn(t, String.prototype.toString)); } catch (e) { r.push(e.name); } try { r.push(callOn(t, String.prototype.valueOf)); } catch (e) { r.push(e.name); } } print(r.join(), String.prototype.valueOf() === '')"#
        ),
        lines(&[
            "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,v,v,w,w true"
        ])
    );
}

#[test]
fn string_functions_have_length_and_name() {
    assert_eq!(
        common::run_both(
            r#"print(String.length, String.name, String.fromCharCode.length, String.prototype.charCodeAt.length, String.prototype.toString.length, String.prototype.valueOf.name, String.prototype.constructor === String, String.prototype.length)"#
        ),
        lines(&["1 String 1 1 0 valueOf true 0"])
    );
}
