//! Array builtins (§23.1): `Array.isArray`, `Array.prototype.pop` and `indexOf`. Every JS check
//! runs with and without GC stress (`common::run_both`).

mod common;

use common::run_both;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn the_functions_are_installed() {
    assert_eq!(
        run_both(
            "print(Array.isArray.name, Array.isArray.length, Array.prototype.pop.length, Array.prototype.indexOf.length);
             var d = Object.getOwnPropertyDescriptor(Array.prototype, 'pop');
             print(d.writable, d.enumerable, d.configurable);
             print(Object.getOwnPropertyNames(Array).join());"
        ),
        lines(&["isArray 1 0 1", "true false true", "length,name,prototype,isArray"])
    );
}

#[test]
fn is_array() {
    assert_eq!(
        run_both(
            "print(Array.isArray([]), Array.isArray([1, 2]), Array.isArray({length: 0}), Array.isArray(Object([])));
             print(Array.isArray(), Array.isArray(undefined), Array.isArray('a'), Array.isArray(new Array(3)));
             class A extends Array {} print(Array.isArray(new A()));"
        ),
        lines(&["true true false true", "false false false true", "true"])
    );
}

#[test]
fn pop_on_arrays() {
    assert_eq!(
        run_both(
            "var a = [1, 2, 3];
             print(a.pop(), a.length, a.join());
             print(a.pop(), a.pop(), a.length, a.pop(), a.length);
             var h = [1, , ]; print(h.length, h.pop(), h.length, h.pop(), h.length);
             var big = []; for (var i = 0; i < 100; i++) big.push(i);
             var s = 0; while (big.length) s += big.pop(); print(s);"
        ),
        lines(&["3 2 1,2", "2 1 0 undefined 0", "2 undefined 1 1 0", "4950"])
    );
}

#[test]
fn pop_is_generic() {
    assert_eq!(
        run_both(
            "var o = {length: 2, 0: 'a', 1: 'b'};
             print(Array.prototype.pop.call(o), o.length, 1 in o, o[0]);
             var e = {}; print(Array.prototype.pop.call(e), e.length);
             var w = {length: '3', 2: 'z'}; print(Array.prototype.pop.call(w), w.length);
             var n = {length: -1}; print(Array.prototype.pop.call(n), n.length);"
        ),
        lines(&["b 1 false a", "undefined 0", "z 2", "undefined 0"])
    );
    for src in [
        "Array.prototype.pop.call(undefined)",
        "Array.prototype.pop.call(null)",
        // A String wrapper's indices and `length` are not configurable or writable.
        "Array.prototype.pop.call('ab')",
    ] {
        let r = run_both(src);
        assert!(
            matches!(&r, Err(m) if m.starts_with("TypeError: ")),
            "{src}: {r:?}"
        );
    }
}

#[test]
fn index_of() {
    assert_eq!(
        run_both(
            "var a = [1, 2, 3, 2];
             print(a.indexOf(2), a.indexOf(4), a.indexOf('2'), a.indexOf(2, 2), a.indexOf(2, -1), a.indexOf(2, -2));
             print(a.indexOf(1, -100), a.indexOf(1, 100), a.indexOf(1, Infinity), a.indexOf(1, -Infinity), a.indexOf(2, 1.9));
             print([NaN].indexOf(NaN), [-0].indexOf(0), [0].indexOf(-0), [1, , 3].indexOf(undefined), [].indexOf(undefined));
             var o = {}; print([1, o].indexOf(o), [1, o].indexOf({}));
             print(Array.prototype.indexOf.call('abc', 'c'), Array.prototype.indexOf.call({length: 3, 2: 'x'}, 'x'));
             print(Array.prototype.indexOf.call({length: 2, 5: 'x'}, 'x'), a.indexOf(3, '2'));"
        ),
        lines(&[
            "1 -1 -1 3 3 3",
            "0 -1 -1 0 1",
            "-1 0 0 -1 -1",
            "1 -1",
            "2 2",
            "-1 2",
        ])
    );
    let r = run_both("Array.prototype.indexOf.call(null, 1)");
    assert!(
        matches!(&r, Err(m) if m.starts_with("TypeError: ")),
        "{r:?}"
    );
}

/// `fromIndex` is converted after `length` (§23.1.3.17 steps 2 and 4), and not when the
/// length is 0 (step 3).
#[test]
fn index_of_converts_in_spec_order() {
    assert_eq!(
        run_both(
            "var log = [];
             var from = {valueOf: function () { log.push('from'); return 0; }};
             var len = {valueOf: function () { log.push('length'); return 2; }};
             var al = {length: len, 0: 'a', 1: 'b'};
             print(Array.prototype.indexOf.call(al, 'b', from), log.join());
             log = [];
             print(Array.prototype.indexOf.call({length: 0}, 'b', from), log.length);"
        ),
        lines(&["1 length,from", "-1 0"])
    );
}

/// A long range scans only the present indices (`present_indices`); the result is the same as
/// the step-by-step loop, including indices on the prototype chain and a range start past the
/// end.
#[test]
fn index_of_over_a_long_sparse_range() {
    assert_eq!(
        run_both(
            "var a = new Array();
             a[100] = 1; a[99999] = ''; a[10] = {}; a[5555] = 5.5; a[123456] = 'str'; a[5] = 1E+309;
             print(a.indexOf(1), a.indexOf(''), a.indexOf('str'), a.indexOf(1E+309), a.indexOf(5.5));
             print(a.indexOf(true), a.indexOf(5), a.indexOf(undefined), a.indexOf(1, 101), a.indexOf('str', -1));
             print(a.indexOf('str', 123457), a.indexOf('str', 1e300), a.indexOf(5.5, -200000));
             var o = {length: 5000, 4000: 'x', 2: 'y', '0400': 'z', 4999.5: 'w'};
             print(Array.prototype.indexOf.call(o, 'x'), Array.prototype.indexOf.call(o, 'y'), Array.prototype.indexOf.call(o, 'z'), Array.prototype.indexOf.call(o, 'w'));
             function P() {} P.prototype[3000] = 'p';
             var q = new P(); q.length = 4000;
             print(Array.prototype.indexOf.call(q, 'p'), Array.prototype.indexOf.call(q, 'p', 3001));"
        ),
        lines(&[
            "100 99999 123456 5 5555",
            "-1 -1 -1 -1 123456",
            "-1 -1 5555",
            "4000 2 -1 -1",
            "3000 -1",
        ])
    );
}

/// The long-range scan reads every own key along the prototype chain: an integer key of
/// 2^32 - 1 or more is not an array index, so `[[OwnPropertyKeys]]` lists it among the other
/// strings in creation order (§10.1.11.1), after `length` or any other key. A non-canonical
/// key ("04294967297") names no index; an own key shadows the prototype's; an accessor in
/// range still takes the step-by-step loop. Lengths reach 2^53 - 1.
#[test]
fn index_of_finds_integer_keys_past_the_array_index_range() {
    assert_eq!(
        run_both(
            "var o = {length: 4294969000}; o[4294967295] = 'hit';
             print(Array.prototype.indexOf.call(o, 'hit', 4294967295), Array.prototype.indexOf.call(o, 'hit', -2000));
             var p = {a: 1, length: 4294969000};
             p[4294967300] = 'y'; p[4294967296] = 'x'; p.z = 2; p[4294967298] = 'x'; p['04294967297'] = 'n'; p['4294967297.0'] = 'n';
             print(Array.prototype.indexOf.call(p, 'x', 4294967290), Array.prototype.indexOf.call(p, 'y', 4294967290), Array.prototype.indexOf.call(p, 'x', 4294967297), Array.prototype.indexOf.call(p, 'n', 4294967290));
             function P() {} P.prototype.b = 1; P.prototype[4294967297] = 'q'; P.prototype[4294967299] = 'r';
             var c = new P(); c.length = 4294969000; c[4294967299] = 'own';
             print(Array.prototype.indexOf.call(c, 'q', 4294967290), Array.prototype.indexOf.call(c, 'r', 4294967290), Array.prototype.indexOf.call(c, 'own', 4294967290));
             var g = {length: 4294969000}; g.k = 1;
             Object.defineProperty(g, 4294967296, {get: function () { return 'got'; }});
             print(Array.prototype.indexOf.call(g, 'got', 4294967290));
             var big = {length: 9007199254740991}; big[9007199254740990] = 'end'; big[9007199254740000] = 'mid';
             print(Array.prototype.indexOf.call(big, 'end', 9007199254739000), Array.prototype.indexOf.call(big, 'mid', -5000));"
        ),
        lines(&[
            "4294967295 4294967295",
            "4294967296 4294967300 4294967298 -1",
            "4294967297 -1 4294967299",
            "4294967296",
            "9007199254740990 9007199254740000",
        ])
    );
}
