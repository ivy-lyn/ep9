//! String (§22.1, Annex B.2.2): `String(v)`, `new String`, `String.fromCharCode`,
//! `String.fromCodePoint`, `String.raw` and the `String.prototype` methods.
//!
//! The expected lines are node 24's output for the same script (`print` = the
//! arguments' ToString joined by spaces), except where a test says ep9 follows the spec and
//! V8 does not. `common::run_both` runs each script with and without GC stress (a collection
//! before every allocation, then the heap verifier). The P4 tests print strings through the
//! `esc!` helpers, which show each code unit outside printable ASCII as `<xxxx>`, so lone
//! surrogates and case mappings show exactly.

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

// ---- P4-B-STRING -----------------------------------------------------------------------------

/// The JS helpers [`esc!`] prepends:
/// - `U(...)` is String.fromCharCode, to write code units (lone surrogates too) by number;
/// - `esc(v)` prints a string with each code unit outside printable ASCII as `<xxxx>` (other
///   values as `type:value`, arrays as `[a,b]`);
/// - `all(a)` joins `esc` of each element of `a` with commas (no Array.prototype.map in
///   ep9_builtins yet);
/// - `name(f)` is String(f()), or the name of the error it throws.
macro_rules! esc {
    ($src:literal) => {
        concat!(
            r#"var U = String.fromCharCode;
            function esc(v) {
                if (typeof v !== 'string') {
                    if (Array.isArray(v)) { return '[' + all(v) + ']'; }
                    return typeof v + ':' + String(v);
                }
                var r = '';
                for (var i = 0; i < v.length; i++) {
                    var c = v.charCodeAt(i);
                    r += (c >= 0x20 && c < 0x7f) ? String.fromCharCode(c) : '<' + (c + 0x10000).toString(16).slice(1) + '>';
                }
                return r;
            }
            function all(a) { var r = []; for (var i = 0; i < a.length; i++) r.push(esc(a[i])); return r.join(); }
            function name(f) { try { return String(f()); } catch (e) { return e.name; } }
            "#,
            $src
        )
    };
}

/// H26: std's case tables are Unicode 17, and the case mapping is the full one (SpecialCasing
/// 'ß' -> "SS") with Final_Sigma.
#[test]
fn acceptance_unicode_17_full_case_mapping_and_final_sigma() {
    assert_eq!(char::UNICODE_VERSION, (17, 0, 0));
    assert_eq!(
        common::run_both(
            "print('ß'.toUpperCase() === 'SS', 'ΑΣ'.toLowerCase() === 'ας', 'ΑΣ Α'.toLowerCase() === 'ας α', 'Σ'.toLowerCase() === 'σ')"
        ),
        lines(&["true true true true"])
    );
}

#[test]
fn string_raw() {
    assert_eq!(
        common::run_both(
            r#"print(String.raw`a${1}b${2}c`, String.raw({ raw: ['x', 'y', 'z'] }, 1), String.raw({ raw: { length: 0 } }) === '', String.raw({ raw: 'abc' }, '-', '+', '*'), String.raw({ raw: { length: 2, 0: 'p', 1: 'q' } }, 9, 8), String.raw({ raw: { length: -5 } }) === ''); try { String.raw(); } catch (e) { print(e.name); } try { String.raw({}); } catch (e) { print(e.name); }"#
        ),
        lines(&["a1b2c x1yz true a-b+c p9q true", "TypeError", "TypeError"])
    );
}

#[test]
fn string_raw_converts_in_order() {
    assert_eq!(
        common::run_both(
            r#"var log = []; var raw = { get length() { log.push('length'); return 2; }, get 0() { log.push('0'); return 'a'; }, get 1() { log.push('1'); return 'b'; } }; var r = String.raw({ get raw() { log.push('raw'); return raw; } }, { toString: function () { log.push('sub0'); return '_'; } }, { toString: function () { log.push('sub1'); return '!'; } }); print(r, log.join())"#
        ),
        lines(&["a_b raw,length,0,sub0,1"])
    );
}

#[test]
fn from_code_point() {
    assert_eq!(
        common::run_both(esc!(
            r#"print(esc(String.fromCodePoint(65, 0x1F600, 0xD800, 0x10FFFF)), esc(String.fromCodePoint()), esc(String.fromCodePoint('65', null)), name(function () { return String.fromCodePoint(1.5); }), name(function () { return String.fromCodePoint(-1); }), name(function () { return String.fromCodePoint(0x110000); }), name(function () { return String.fromCodePoint(NaN); }), name(function () { return String.fromCodePoint(Infinity); }))"#
        )),
        lines(&[
            "A<d83d><de00><d800><dbff><dfff>  A<0000> RangeError RangeError RangeError RangeError RangeError"
        ])
    );
}

#[test]
fn at_char_at_and_code_point_at() {
    assert_eq!(
        common::run_both(esc!(
            r#"print(all(['abc'.at(0), 'abc'.at(-1), 'abc'.at(3), 'abc'.at(-4), 'abc'.at(Infinity), 'abc'.at(-0), 'abc'.at('1')])); print(all(['abc'.charAt(1), 'abc'.charAt(-1), 'abc'.charAt(3), 'abc'.charAt(NaN), 'abc'.charAt()])); print(U(0xD83D, 0xDE00).codePointAt(0), U(0xD83D, 0xDE00).codePointAt(1), U(0xD83D, 0x78).codePointAt(0), 'a'.codePointAt(1), U(0xDE00, 0xD83D).codePointAt(0), U(0x61, 0xD83D).codePointAt(1))"#
        )),
        lines(&[
            "a,c,undefined:undefined,undefined:undefined,undefined:undefined,a,b",
            "b,,,a,a",
            "128512 56832 55357 undefined 56832 55357"
        ])
    );
}

#[test]
fn concat() {
    assert_eq!(
        common::run_both(
            r#"print('a'.concat(1, null, undefined, {}, [2, 3]), 'a'.concat(), String.prototype.concat.call(1, 2), String.prototype.concat.length); try { 'a'.concat(Symbol()); } catch (e) { print(e.name); }"#
        ),
        lines(&["a1nullundefined[object Object]2,3 a 12 1", "TypeError"])
    );
}

#[test]
fn index_of_and_last_index_of() {
    assert_eq!(
        common::run_both(
            r#"var U = String.fromCharCode; print('abcabc'.indexOf('c'), 'abcabc'.indexOf('c', 3), 'abc'.indexOf(''), 'abc'.indexOf('', 5), 'abc'.indexOf('d'), 'aaa'.indexOf('aa', 1), 'abc'.indexOf('c', -5), 'abc'.indexOf('abcd'), U(0x61, 0xD800, 0x62).indexOf(U(0xD800))); print('abcabc'.lastIndexOf('c'), 'abcabc'.lastIndexOf('c', 4), 'abc'.lastIndexOf(''), 'abc'.lastIndexOf('', 1), 'abc'.lastIndexOf('abcd'), 'aaa'.lastIndexOf('aa'), 'abc'.lastIndexOf('a', -Infinity), 'abc'.lastIndexOf('c', NaN), ''.lastIndexOf(''), 'abc'.lastIndexOf('c', undefined))"#
        ),
        lines(&["2 5 0 3 -1 1 2 -1 1", "5 2 3 1 -1 1 0 2 0 2"])
    );
}

#[test]
fn includes_starts_with_and_ends_with() {
    assert_eq!(
        common::run_both(
            r#"print('abc'.includes('b'), 'abc'.includes('b', 2), 'abc'.includes(''), 'abc'.includes('', 10), ''.includes('')); print('abc'.startsWith('ab'), 'abc'.startsWith('b', 1), 'abc'.startsWith('', 9), 'abc'.startsWith('c', 3), 'abc'.startsWith('a', -5), 'abc'.startsWith('abcd')); print('abc'.endsWith('bc'), 'abc'.endsWith('b', 2), 'abc'.endsWith(''), 'abc'.endsWith('abcd'), 'abc'.endsWith('a', -1), 'abc'.endsWith('c', Infinity))"#
        ),
        lines(&[
            "true false true true true",
            "true true true false true false",
            "true true true false false true"
        ])
    );
}

/// IsRegExp (§7.2.8) reads @@match: a truthy one makes includes, startsWith and endsWith
/// throw; a falsy one makes the object a plain search string.
#[test]
fn search_methods_reject_a_reg_exp_by_symbol_match() {
    assert_eq!(
        common::run_both(esc!(
            r#"var re = {}; re[Symbol.match] = true; var not = { toString: function () { return 'b'; } }; not[Symbol.match] = false; print(name(function () { return 'a'.includes(re); }), name(function () { return 'a'.startsWith(re); }), name(function () { return 'a'.endsWith(re); }), 'abc'.includes(not), 'bc'.startsWith(not), 'ab'.endsWith(not)); var log = []; var getter = {}; Object.defineProperty(getter, Symbol.match, { get: function () { log.push('match'); return undefined; } }); getter.toString = function () { log.push('toString'); return 'x'; }; print('x'.includes(getter), log.join())"#
        )),
        lines(&[
            "TypeError TypeError TypeError true true true",
            "true match,toString"
        ])
    );
}

#[test]
fn is_well_formed_and_to_well_formed() {
    assert_eq!(
        common::run_both(esc!(
            r#"print('ab'.isWellFormed(), U(0x61, 0xD800).isWellFormed(), U(0xD83D, 0xDE00).isWellFormed(), U(0xDE00, 0xD83D).isWellFormed(), ''.isWellFormed()); print(esc(U(0x61, 0xD800, 0x62, 0xDE00, 0xD83D, 0xDE00, 0xD83D).toWellFormed()), esc('ok'.toWellFormed()))"#
        )),
        lines(&[
            "true false true false true",
            "a<fffd>b<fffd><d83d><de00><fffd> ok"
        ])
    );
}

/// v0: code-unit order (no ECMA-402); node agrees on these inputs.
#[test]
fn locale_compare_is_code_unit_order() {
    assert_eq!(
        common::run_both(
            r#"print('a'.localeCompare('b'), 'b'.localeCompare('a'), 'a'.localeCompare('a'), 'a'.localeCompare('ab'), ''.localeCompare(''), 'a'.localeCompare(), 'undefined'.localeCompare())"#
        ),
        lines(&["-1 1 0 -1 0 -1 0"])
    );
}

#[test]
fn pad_start_and_pad_end() {
    assert_eq!(
        common::run_both(esc!(
            r#"print(all(['abc'.padStart(6), 'abc'.padEnd(6, '12'), 'abc'.padStart(8, 'xy'), 'abc'.padStart(2), 'abc'.padEnd(10, ''), 'abc'.padStart(-1, 'z'), 'a'.padEnd(3, undefined), 'a'.padStart(3, null), 'a'.padEnd(Math.pow(2, 53) - 1, '')])); print(name(function () { return 'a'.padEnd(Math.pow(2, 53) - 1, 'b'); }), name(function () { return 'a'.padStart(Math.pow(2, 29), 'b'); }))"#
        )),
        lines(&[
            "   abc,abc121,xyxyxabc,abc,abc,abc,a  ,nua,a",
            "RangeError RangeError"
        ])
    );
}

/// The length check comes before any allocation: 2^28 copies of "ab" (2^29 code units) pass
/// the VM limit (2^29 - 24) and throw at once.
#[test]
fn repeat() {
    assert_eq!(
        common::run_both(esc!(
            r#"print(all(['ab'.repeat(3), 'ab'.repeat(0), ''.repeat(Math.pow(2, 40)), 'x'.repeat(2.9), 'x'.repeat(NaN), 'x'.repeat('2')])); print(name(function () { return 'a'.repeat(-1); }), name(function () { return 'a'.repeat(Infinity); }), name(function () { return ''.repeat(Infinity); }), name(function () { return 'ab'.repeat(Math.pow(2, 28)); }), name(function () { return 'a'.repeat(Math.pow(2, 29) - 23); }))"#
        )),
        lines(&[
            "ababab,,,xx,,xx",
            "RangeError RangeError RangeError RangeError RangeError"
        ])
    );
}

#[test]
fn slice_substring_and_substr() {
    assert_eq!(
        common::run_both(esc!(
            r#"print(all(['abcdef'.slice(1, 3), 'abcdef'.slice(-2), 'abcdef'.slice(2, -1), 'abcdef'.slice(4, 2), 'abcdef'.slice(-Infinity, Infinity), 'abc'.slice(), 'abc'.slice(1, undefined)])); print(all(['abcdef'.substring(1, 3), 'abcdef'.substring(3, 1), 'abcdef'.substring(-1), 'abcdef'.substring(NaN, 2), 'abcdef'.substring(2, Infinity)])); print(all(['abcdef'.substr(1, 3), 'abcdef'.substr(-2), 'abcdef'.substr(-10, 2), 'abcdef'.substr(2), 'abcdef'.substr(4, -1), 'abcdef'.substr(NaN, Infinity), 'abc'.substr(5, 1), 'abc'.substr(-Infinity, 2)]))"#
        )),
        lines(&[
            "bc,ef,cde,,abcdef,abc,bc",
            "bc,bc,abcdef,ab,cdef",
            "bcd,ef,ab,cdef,,abcdef,,ab"
        ])
    );
}

/// Full mappings (SpecialCasing), Final_Sigma with Case_Ignorable code points around Σ
/// (U+180E, U+02B0, '.'), and lone surrogates kept: a lone surrogate is neither cased nor
/// case-ignorable, so it ends the Σ context. Lines from node 24 (ICU), which agrees with Rust
/// std here.
#[test]
fn case_mapping() {
    assert_eq!(
        common::run_both(esc!(
            r#"var S = U(0x3A3); print(esc(('ABC ' + U(0x130) + ' ' + S + ' A' + S + ' A' + S + 'B ' + U(0x391) + S + ' x' + U(0xD800) + S + ' ' + U(0x1E9E)).toLowerCase())); print(esc(('abc ' + U(0xDF) + ' ' + U(0xFB00) + ' ' + U(0x149) + ' ' + U(0x390) + ' ' + U(0xD801, 0xDC28) + ' ' + U(0xD800) + 'x i').toUpperCase())); var sig = ['A' + S, 'A' + S + U(0x180E), 'A' + S + U(0x180E) + 'B', S, 'A.' + S, 'A' + S + '.b', 'A' + U(0xD800) + S, 'A' + S + U(0xD800) + 'b', U(0x2B0) + S, 'A' + U(0x2B0) + S, 'A' + S + U(0x2B0) + 'b']; var r = []; for (var i = 0; i < sig.length; i++) r.push(sig[i].toLowerCase()); print(all(r)); print(esc(('I' + U(0x130) + 'i').toLocaleLowerCase()), esc(('i' + U(0x131)).toLocaleUpperCase()), String.prototype.toUpperCase.call(true), String.prototype.toLowerCase.call(1e21))"#
        )),
        lines(&[
            "abc i<0307> <03c3> a<03c2> a<03c3>b <03b1><03c2> x<d800><03c3> <00df>",
            "ABC SS FF <02bc>N <0399><0308><0301> <d801><dc00> <d800>X I",
            "a<03c2>,a<03c2><180e>,a<03c3><180e>b,<03c3>,a.<03c2>,a<03c3>.b,a<d800><03c3>,a<03c2><d800>b,<02b0><03c3>,a<02b0><03c2>,a<03c3><02b0>b",
            "ii<0307>i II TRUE 1e+21"
        ])
    );
}

/// WhiteSpace and LineTerminator (§12.2, §12.3): U+FEFF is one, U+0085 and U+180E are not.
#[test]
fn trim_methods_and_their_annex_b_aliases() {
    assert_eq!(
        common::run_both(esc!(
            r#"var ws = U(0x20, 0x9, 0xA, 0xB, 0xC, 0xD, 0xA0, 0x1680, 0x2000, 0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF); print(all([ws + 'a b' + ws, '  a  '.trimStart(), '  a  '.trimEnd(), U(0x85) + 'a' + U(0x85), U(0x180E) + 'a' + U(0x180E), '   ', '  ', '  ', ''])); print(all([(ws + 'a b' + ws).trim(), '  a  '.trimStart(), '  a  '.trimEnd(), (U(0x85) + 'a' + U(0x85)).trim(), (U(0x180E) + 'a' + U(0x180E)).trim(), '   '.trim(), '  '.trimStart(), '  '.trimEnd(), ''.trim()])); print(String.prototype.trimLeft === String.prototype.trimStart, String.prototype.trimRight === String.prototype.trimEnd, String.prototype.trimLeft.name, String.prototype.trimRight.name, ' x '.trimLeft() + '|', ' x '.trimRight() + '|')"#
        )),
        lines(&[
            " <0009><000a><000b><000c><000d><00a0><1680><2000><200a><2028><2029><202f><205f><3000><feff>a b <0009><000a><000b><000c><000d><00a0><1680><2000><200a><2028><2029><202f><205f><3000><feff>,a  ,  a,<0085>a<0085>,<180e>a<180e>,   ,  ,  ,",
            "a b,a  ,  a,<0085>a<0085>,<180e>a<180e>,,,,",
            "true true trimStart trimEnd x |  x|"
        ])
    );
}

#[test]
fn split_with_string_separators() {
    assert_eq!(
        common::run_both(esc!(
            r#"print(esc('a,b,c'.split(',')), esc('a,b,c'.split(',', 2)), esc('abc'.split('')), esc('abc'.split('', 2)), esc(''.split(',')), esc(''.split('')), esc('abc'.split()), esc('abc'.split(undefined, 0)), esc('a,,b'.split(',')), esc('abc'.split('abc')), esc('aXbXc'.split('X', -1)), esc('abc'.split(',', 0)), esc('a,b,c'.split(',', 4294967297)), esc('a1b1c'.split(1)), esc(U(0xD83D, 0xDE00).split('')))"#
        )),
        lines(&[
            "[a,b,c] [a,b] [a,b,c] [a,b] [] [] [abc] [] [a,,b] [,] [a,b,c] [] [a] [a,b,c] [<d83d>,<de00>]"
        ])
    );
}

/// CreateArrayFromList defines the elements: a setter on Array.prototype is not called.
#[test]
fn split_defines_its_elements() {
    assert_eq!(
        common::run_both(
            r#"var hit = false; Object.defineProperty(Array.prototype, '0', { set: function () { hit = true; }, configurable: true }); var r = 'x,y'.split(','); print(r.length, r[0], r[1], hit, Object.getOwnPropertyDescriptor(r, '0').writable)"#
        ),
        lines(&["2 x y false true"])
    );
}

#[test]
fn replace_and_replace_all_with_string_patterns() {
    assert_eq!(
        common::run_both(esc!(
            r#"print(all(['abcabc'.replace('b', 'X'), 'abc'.replace('d', 'X'), 'abc'.replace('', '_'), 'abc'.replace('b', '[$&|$`|$\'|$$|$1|$01|$<n>|$]'), 'abc'.replace('b', function (m, p, s) { return m + p + s; }), 'aaa'.replace('a', '$'), 'abc'.replace('c', '$\'.'), 'a1b'.replace(1, 2)])); print(all(['abcabc'.replaceAll('b', 'X'), 'abc'.replaceAll('', '_'), 'aaaa'.replaceAll('aa', 'b'), 'abcb'.replaceAll('b', '[$`|$\'|$&]'), 'xyx'.replaceAll('x', function (m, p) { return p; }), ''.replaceAll('', 'e'), 'abc'.replaceAll('z', 'y')]))"#
        )),
        lines(&[
            "aXcabc,abc,_abc,a[b|a|c|$|$1|$01|$<n>|$]c,ab1abcc,$aa,ab.,a2b",
            "aXcaXc,_a_b_c_,bb,a[a|cb|b]c[abc||b],0y2,e,abc"
        ])
    );
}

/// An object pattern with a @@split / @@replace method is called with (O, the second
/// argument); a non-callable one is a TypeError; replaceAll checks the "g" flag of an
/// IsRegExp pattern first.
#[test]
fn split_and_replace_call_the_pattern_symbol_methods() {
    assert_eq!(
        common::run_both(esc!(
            r#"var o = {}; o[Symbol.replace] = function (s, r) { return [this === o, s, r].join('/'); }; o[Symbol.split] = function (s, l) { return [this === o, s, l].join('/'); }; print('abc'.replace(o, 'R'), 'abc'.replaceAll(o, 'S'), 'abc'.split(o, 7), String.prototype.split.call(5, o)); var bad = {}; bad[Symbol.split] = 1; bad[Symbol.replace] = 'x'; print(name(function () { return 'a'.split(bad); }), name(function () { return 'a'.replace(bad); }), name(function () { return 'a'.replaceAll(bad); })); var nul = { toString: function () { return 'b'; } }; nul[Symbol.replace] = null; nul[Symbol.split] = null; print('abc'.replace(nul, '_'), esc('abc'.split(nul))); var reI = { flags: 'i' }; reI[Symbol.match] = true; var reG = { flags: 'g' }; reG[Symbol.match] = true; reG[Symbol.replace] = function (s, r) { return 'custom:' + s + ':' + r; }; var reU = {}; reU[Symbol.match] = true; print(name(function () { return 'a'.replaceAll(reI, 'b'); }), 'a'.replaceAll(reG, 'b'), name(function () { return 'a'.replaceAll(reU, 'b'); }))"#
        )),
        lines(&[
            "true/abc/R true/abc/S true/abc/7 true/5/",
            "TypeError TypeError TypeError",
            "a_c [a,c]",
            "TypeError custom:a:b TypeError"
        ])
    );
}

/// ep9 follows the spec (ecma262 PR 3009; test262 `cstm-replace-on-*-primitive.js`): the
/// pattern's @@replace / @@split is read only when the pattern is an Object. node 24 (V8)
/// still reads it from a primitive's prototype and prints "threw read" here.
#[test]
fn a_primitive_pattern_is_not_asked_for_its_symbol_method() {
    assert_eq!(
        common::run_both(
            r#"Object.defineProperty(Number.prototype, Symbol.replace, { get: function () { throw new Error('read'); }, configurable: true }); Object.defineProperty(String.prototype, Symbol.split, { get: function () { throw new Error('read'); }, configurable: true }); try { print('a1b'.replace(1, 'X'), 'a,b'.split(',').length, 'a1b1'.replaceAll(1, '_')); } catch (e) { print('threw', e.message); }"#
        ),
        lines(&["aXb 2 a_b_"])
    );
}

#[test]
fn html_methods() {
    assert_eq!(
        common::run_both(esc!(
            r#"print(all(['a'.anchor('x"y'), 'a'.big(), 'a'.blink(), 'a'.bold(), 'a'.fixed(), 'a'.fontcolor('red'), 'a'.fontsize(7), 'a'.italics(), 'a'.link('u'), 'a'.small(), 'a'.strike(), 'a'.sub(), 'a'.sup(), 'a'.anchor(), String.prototype.bold.call(1)])); print(name(function () { return String.prototype.big.call(null); }), name(function () { return 'a'.link(Symbol()); }))"#
        )),
        lines(&[
            r#"<a name="x&quot;y">a</a>,<big>a</big>,<blink>a</blink>,<b>a</b>,<tt>a</tt>,<font color="red">a</font>,<font size="7">a</font>,<i>a</i>,<a href="u">a</a>,<small>a</small>,<strike>a</strike>,<sub>a</sub>,<sup>a</sup>,<a name="undefined">a</a>,<b>1</b>"#,
            "TypeError TypeError"
        ])
    );
}

/// RequireObjectCoercible, then ToString of this, then the arguments in order (split
/// converts limit before separator, §22.1.3.23 steps 4-5).
#[test]
fn methods_convert_this_then_the_arguments_in_order() {
    assert_eq!(
        common::run_both(
            r#"function logger(log, tag, v) { return { toString: function () { log.push(tag); return v; }, valueOf: function () { log.push(tag); return v; } }; } var cases = [['indexOf', ['b', 0]], ['padStart', [5, 'x']], ['split', ['b', 5]], ['replace', ['b', 'x']], ['replaceAll', ['b', 'x']], ['slice', [0, 1]], ['substr', [0, 1]], ['anchor', ['n']], ['includes', ['b', 0]], ['lastIndexOf', ['b', 0]]]; for (var i = 0; i < cases.length; i++) { var log = []; var a = cases[i][1]; String.prototype[cases[i][0]].call(logger(log, 'this', 'abc'), logger(log, 'a0', a[0]), logger(log, 'a1', a[1])); print(cases[i][0], log.join()); } var names = ['at', 'charAt', 'codePointAt', 'concat', 'endsWith', 'includes', 'indexOf', 'lastIndexOf', 'isWellFormed', 'toWellFormed', 'localeCompare', 'padStart', 'padEnd', 'repeat', 'slice', 'split', 'startsWith', 'substring', 'toLowerCase', 'toUpperCase', 'toLocaleLowerCase', 'toLocaleUpperCase', 'trim', 'trimStart', 'trimEnd', 'replace', 'replaceAll', 'substr', 'big']; var bad = []; for (var j = 0; j < names.length; j++) { try { String.prototype[names[j]].call(undefined); bad.push(names[j]); } catch (e) { if (!(e instanceof TypeError)) bad.push(names[j]); } } print('null this:', bad.length)"#
        ),
        lines(&[
            "indexOf this,a0,a1",
            "padStart this,a0,a1",
            "split this,a1,a0",
            "replace this,a0,a1",
            "replaceAll this,a0,a1",
            "slice this,a0,a1",
            "substr this,a0,a1",
            "anchor this,a0",
            "includes this,a0,a1",
            "lastIndexOf this,a0,a1",
            "null this: 0"
        ])
    );
}

#[test]
fn new_functions_have_length_and_name() {
    assert_eq!(
        common::run_both(
            r#"var names = ['at', 'charAt', 'codePointAt', 'concat', 'endsWith', 'includes', 'indexOf', 'lastIndexOf', 'isWellFormed', 'toWellFormed', 'localeCompare', 'padStart', 'padEnd', 'repeat', 'slice', 'split', 'startsWith', 'substring', 'toLowerCase', 'toUpperCase', 'toLocaleLowerCase', 'toLocaleUpperCase', 'trim', 'trimStart', 'trimEnd', 'replace', 'replaceAll', 'substr', 'anchor', 'big', 'blink', 'bold', 'fixed', 'fontcolor', 'fontsize', 'italics', 'link', 'small', 'strike', 'sub', 'sup']; var r = []; for (var i = 0; i < names.length; i++) { var f = String.prototype[names[i]]; var d = Object.getOwnPropertyDescriptor(String.prototype, names[i]); r.push(f.length + (f.name === names[i] ? '' : '!') + (d.writable && !d.enumerable && d.configurable ? '' : '?')); } print(r.join(' ')); print(String.raw.length, String.raw.name, String.fromCodePoint.length, String.fromCodePoint.name)"#
        ),
        lines(&[
            "1 1 1 1 1 1 1 1 0 0 1 1 1 1 2 2 1 2 0 0 0 0 0 0 0 2 2 2 1 0 0 0 0 1 1 0 1 0 0 0 0",
            "1 raw 1 fromCodePoint"
        ])
    );
}

/// Loops that make a string per pass (split, replaceAll) or can run to 2^53 - 1 (String.raw)
/// take one interrupt budget unit per pass (`Cx::poll_interrupt`): termination stops them.
/// Termination is requested before the script runs, and nothing polls between its start and
/// the loop (`repeat` makes the input without a loop poll), so only a poll in the loop stops
/// it once the budget (10,000 units) runs out. Without the poll, the String.raw case ends in
/// a RangeError (the string length limit) and the others complete. The 60 s wait only stops
/// a hung test.
#[test]
fn termination_stops_the_string_loops() {
    let cases = [
        "String.raw({ raw: { length: 2 ** 53 - 1 } })",
        "'a'.repeat(100000).split('')",
        "'ab'.repeat(100000).split('b')",
        "'a'.repeat(100000).replaceAll('a', 'b')",
    ];
    for call in cases {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(move || {
                let mut iso = common::isolate(false);
                let unit = |src: &str| {
                    ep9_frontend::compile(
                        src,
                        ep9_frontend::SourceOpts::script(ep9_frontend::Strictness::Sloppy),
                        ep9_frontend::CompileOptions::default(),
                    )
                    .expect("compiles")
                };
                iso.terminate_handle().terminate();
                let src = format!("var stage = 'before'; var r = {call}; stage = 'reached';");
                let r = iso.with_cx(|cx| cx.run_script(unit(&src), "call.js").map(|_| ()));
                let terminated = r.is_err_and(|e| e.terminated);
                let stage = |iso: &mut ep9_vm::Isolate| {
                    iso.with_cx(|cx| {
                        let v = cx.global_binding("stage")?;
                        Ok(v.map(|v| cx.display(v)))
                    })
                    .expect("reads")
                };
                let before = stage(&mut iso);
                // The isolate keeps working.
                iso.with_cx(|cx| {
                    cx.run_script(unit("stage = 'again'"), "again.js")
                        .map(|_| ())
                })
                .expect("runs");
                let _ = tx.send((terminated, before, stage(&mut iso)));
            })
            .expect("a thread");
        let got = rx.recv_timeout(std::time::Duration::from_secs(60));
        assert_eq!(
            got,
            Ok((true, Some("before".to_string()), Some("again".to_string()))),
            "{call}"
        );
    }
}
