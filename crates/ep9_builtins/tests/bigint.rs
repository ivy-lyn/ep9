//! BigInt (§21.2, spec-temporal B4): the constructor, `asIntN`, `asUintN`, the prototype
//! methods, the operators, comparisons, conversions, Map/Set keys, `switch`, `Number()` and
//! `JSON.stringify` with BigInts.
//!
//! The expected lines are node v26.13.0's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn bigint_called_converts() {
    assert_eq!(
        common::run_both(
            r#"print(BigInt(0), BigInt(-1.0), BigInt(2**60), BigInt(true), BigInt(false), BigInt(" 0x1F "), BigInt("-12"), BigInt(""), BigInt("0b101"), BigInt("0o17"), BigInt(12n), BigInt({valueOf() { return 7; }}), typeof BigInt(1))"#
        ),
        lines(&["0 -1 1152921504606846976 1 0 31 -12 0 5 15 12 7 bigint"])
    );
}

#[test]
fn bigint_constructor_errors() {
    assert_eq!(
        common::run_both(
            r#"var r = []; for (var v of [1.5, NaN, Infinity, undefined, null, Symbol(), "1n", "1.5", "0x", "-0x1", "1_0", "1e3"]) { try { BigInt(v); r.push('ok'); } catch (e) { r.push(e.constructor.name); } } try { new BigInt(1); } catch (e) { r.push(e.constructor.name); } print(r.join())"#
        ),
        lines(&[
            "RangeError,RangeError,RangeError,TypeError,TypeError,TypeError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,TypeError"
        ])
    );
}

#[test]
fn as_int_n_and_as_uint_n() {
    assert_eq!(
        common::run_both(
            r#"print(BigInt.asIntN(64, 2n ** 63n), BigInt.asUintN(64, -1n), BigInt.asIntN(8, 255n), BigInt.asUintN(8, 257n), BigInt.asIntN(0, 5n), BigInt.asIntN(1, -1n), BigInt.asUintN(3, "9"), BigInt.asIntN("8", true)); var r = []; for (var b of [-1, 2**53, Infinity]) { try { BigInt.asIntN(b, 1n); r.push('ok'); } catch (e) { r.push(e.constructor.name); } } try { BigInt.asUintN(1, 1); } catch (e) { r.push(e.constructor.name); } print(r.join())"#
        ),
        lines(&[
            "-9223372036854775808 18446744073709551615 -1 1 0 -1 1 1",
            "RangeError,RangeError,RangeError,TypeError"
        ])
    );
}

#[test]
fn prototype_methods_and_wrappers() {
    assert_eq!(
        common::run_both(
            r#"print((255n).toString(16), (-255n).toString(2), (10n ** 20n).toString(36), 0n.toString(), (123n).toLocaleString(), Object(5n).valueOf(), typeof Object(5n), Object(5n) + 1n, Object.prototype.toString.call(1n), BigInt.prototype[Symbol.toStringTag], BigInt.prototype.constructor === BigInt, Object.getPrototypeOf(1n) === BigInt.prototype, 1n.constructor === BigInt); var r = []; for (var x of [2, 37, 0]) { try { 5n.toString(x); r.push('ok'); } catch (e) { r.push(e.constructor.name); } } for (var t of [1, 'a', {}, Object(1)]) { try { BigInt.prototype.valueOf.call(t); r.push('ok'); } catch (e) { r.push(e.constructor.name); } } print(r.join())"#
        ),
        lines(&[
            "ff -11111111 l3r41ifs0q5ts 0 123 5 object 6 [object BigInt] BigInt true true true",
            "ok,RangeError,RangeError,TypeError,TypeError,TypeError,TypeError"
        ])
    );
}

#[test]
fn operators() {
    assert_eq!(
        common::run_both(
            r#"var a = 2n ** 64n; print(a + 1n, a - 1n, a * a, a / 3n, -a % 7n, a >> 3n, -a >> 70n, 1n << 70n, -5n & 3n, -5n | 3n, -5n ^ 3n, ~a, -a, a ** 0n); var r = []; for (var f of [() => 1n + 1, () => 1n >>> 0n, () => 1n / 0n, () => 2n ** -1n, () => +1n, () => Math.abs(1n), () => 1n | 1]) { try { f(); r.push('ok'); } catch (e) { r.push(e.constructor.name); } } print(r.join())"#
        ),
        lines(&[
            "18446744073709551617 18446744073709551615 340282366920938463463374607431768211456 6148914691236517205 -2 2305843009213693952 -1 1180591620717411303424 3 -5 -8 -18446744073709551617 -18446744073709551616 1",
            "TypeError,TypeError,RangeError,RangeError,TypeError,TypeError,TypeError"
        ])
    );
}

#[test]
fn comparisons_and_keys() {
    assert_eq!(
        common::run_both(
            r#"print(1n == 1, 1n === 1, 1n == "1", 2n > 1, 1n < 1.5, 2n ** 64n == 18446744073709551616, 1n == true, 0n == "", 1n < "x", 1n >= "x", [1n, 2n].includes(1n), [0n].indexOf(0n), Object.is(0n, -0n), new Set([1n, 1n, 2n]).size, new Map([[10n ** 30n, 'a']]).get(10n ** 30n))"#
        ),
        lines(&["true false true true true true true true false false true 0 true 2 a"])
    );
}

/// A property key, `++`/`--`/`+=`, `switch` (strict equality of two cells with one value),
/// truthiness, template literals, String(), Number() (correctly rounded) and sort.
#[test]
fn keys_switch_and_conversions() {
    assert_eq!(
        common::run_both(
            r#"var o = {}; o[1n] = 'one'; var x = 5n; x++; x--; x += 2n; var s; switch (3n * 2n) { case 6: s = 'num'; break; case 6n: s = 'big'; break; } print(o["1"], x, typeof x, !0n, !!1n, `${10n}`, String(-3n), Number(2n ** 64n), Number(-7n), Number(2n ** 1024n), Number(2n ** 53n + 1n), s, parseInt("12n"), [3n, 1n, 2n].sort().join(), 10n > 9)"#
        ),
        lines(&[
            "one 7 bigint true true 10 -3 18446744073709552000 -7 Infinity 9007199254740992 big 12 1,2,3 true"
        ])
    );
}

/// SerializeJSONProperty: a BigInt member, element, value or wrapper is a TypeError (step 10;
/// a member is not skipped as undefined), and `toJSON` on `BigInt.prototype` runs (step 2).
#[test]
fn json_stringify_of_bigints() {
    assert_eq!(
        common::run_both(
            r#"var r = []; for (var v of [{a: 1n}, [1n], 1n, Object(1n), {a: Object(2n)}]) { try { r.push(JSON.stringify(v)); } catch (e) { r.push(e.constructor.name); } } BigInt.prototype.toJSON = function () { return this.toString() + 'n'; }; r.push(JSON.stringify({a: 3n, b: [4n]})); delete BigInt.prototype.toJSON; print(r.join(' '))"#
        ),
        lines(&[r#"TypeError TypeError TypeError TypeError TypeError {"a":"3n","b":["4n"]}"#])
    );
}

/// The size limit and the string length limit are RangeErrors that a script catches.
#[test]
fn size_limits_are_range_errors() {
    assert_eq!(
        common::run_both(
            r#"var r = []; for (var f of [() => 1n << (2n ** 30n), () => 2n ** (2n ** 30n), () => (1n << 600000000n).toString(2), () => BigInt.asUintN(2 ** 31, -1n)]) { try { f(); r.push('ok'); } catch (e) { r.push(e.constructor.name + ': ' + e.message); } } print(r.join('; '))"#
        ),
        lines(&[
            "RangeError: Maximum BigInt size exceeded; RangeError: Maximum BigInt size exceeded; RangeError: Invalid string length; RangeError: Maximum BigInt size exceeded"
        ])
    );
}
