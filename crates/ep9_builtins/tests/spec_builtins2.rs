//! M7-P3 SPEC-BUILTINS2: the built-ins fixes of the V12-D25 pick and route JS3-VALIDATOR.
//!
//! The expected lines are node's output for the same script (`print` = the arguments'
//! ToString joined by spaces). `common::run_both` runs each script with and without GC
//! stress (a collection before every allocation, then the heap verifier). The scripts
//! compare local-time values with `new Date(y, m, d)`, so they hold in every time zone.

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Route JS3-VALIDATOR: Date.parse takes the numeric slash dates "M/D/Y" and "Y/M/D" (the
/// §21.4.3.2 fallback; V8's legacy parser). JetStream validatorjs `isBefore` compares with
/// "08/04/2011".
#[test]
fn date_parse_takes_slash_dates() {
    assert_eq!(
        common::run_both(
            r#"
            var d = new Date(2011, 7, 4).getTime();
            print(Date.parse("08/04/2011") === d, Date.parse("8/4/2011") === d,
                Date.parse("2011/08/04") === d, Date.parse("08/04/11") === d,
                Date.parse("Thu 08/04/2011") === d);
            print(Date.parse("08/04/2011 10:00:30") === new Date(2011, 7, 4, 10, 0, 30).getTime(),
                Date.parse("8/4/99") === new Date(1999, 7, 4).getTime());
            print(new Date("08/04/2011 10:00 GMT").toISOString(),
                new Date("12/31/1969 23:59:59 GMT-0100").toISOString());
            print(Date.parse("13/04/2011"), Date.parse("08/32/2011"), Date.parse("08/04/-2011"),
                Date.parse("8/4/2011 24:30"));
            // validatorjs isBefore with the comparison date "08/04/2011".
            var before = function (s, c) {
                var a = Date.parse(s), b = Date.parse(c);
                return !isNaN(a) && !isNaN(b) && a < b;
            };
            print(before("2010-07-02", "08/04/2011"), before("08/04/2011", "08/04/2011"));
            "#
        ),
        lines(&[
            "true true true true true",
            "true true",
            "2011-08-04T10:00:00.000Z 1970-01-01T00:59:59.000Z",
            "NaN NaN NaN NaN",
            "true false",
        ])
    );
}

/// Number.prototype.toLocaleString (§21.1.3.4): {w, c}, length 0, thisNumberValue; with no
/// ECMA-402 it gives the `toString()` text (as BigInt's). node with ICU gives the same lines
/// for these values (it prints "1,234.5" for 1234.5, so the test uses no such value).
#[test]
fn number_to_locale_string() {
    assert_eq!(
        common::run_both(
            r#"
            var P = Number.prototype, d = Object.getOwnPropertyDescriptor(P, "toLocaleString");
            print(typeof d.value, d.writable, d.enumerable, d.configurable, d.value.length, d.value.name);
            print((12).toLocaleString(), new Number(-7).toLocaleString(), NaN.toLocaleString());
            print(Object.getOwnPropertyNames(P).join());
            try { P.toLocaleString.call("1"); } catch (e) { print(e.name); }
            try { new P.toLocaleString(); } catch (e) { print(e.name); }
            "#
        ),
        lines(&[
            "function true false true 0 toLocaleString",
            "12 -7 NaN",
            "constructor,toExponential,toFixed,toPrecision,toString,valueOf,toLocaleString",
            "TypeError",
            "TypeError",
        ])
    );
}

/// %Array.prototype% is an Array exotic object (§23.1.3): Array.isArray, `length` {w},
/// index writes move `length`, and a proxy of an object that inherits from it has `length`
/// (test262 built-ins/Array/prototype/exotic-array.js, Proxy/has/trap-is-undefined.js).
#[test]
fn array_prototype_is_an_array() {
    assert_eq!(
        common::run_both(
            r#"
            print(Array.isArray(Array.prototype), Array.prototype.length, Object.prototype.toString.call(Array.prototype));
            var d = Object.getOwnPropertyDescriptor(Array.prototype, "length");
            print(d.value, d.writable, d.enumerable, d.configurable, Object.getPrototypeOf(Array.prototype) === Object.prototype);
            Array.prototype[2] = 42;
            print(Array.prototype.length, [].length, [][2], [5].concat([6]).length, [1, , 3][1]);
            Array.prototype.length = 0;
            print(Array.prototype[2], [][2], Array.prototype.length);
            var o = Object.create(Array.prototype);
            print("length" in new Proxy(o, {}), "foo" in new Proxy(o, {}), o.length, Array.isArray(o));
            "#
        ),
        lines(&[
            "true 0 [object Array]",
            "0 true false false true",
            "3 0 42 2 undefined",
            "undefined undefined 0",
            "true false 0 false",
        ])
    );
}
