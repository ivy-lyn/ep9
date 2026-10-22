//! Array builtins (§23.1): `Array.isArray`, `Array.prototype.pop` and `indexOf` (week 4), and
//! the rest of §23.1 with `Object.fromEntries` and `Object.groupBy` (M2-P4 B-ARRAY). Every JS
//! check runs with and without GC stress (`common::run_both`); the expected lines are node
//! v24.22.1's output for the same script.

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
        lines(&[
            "isArray 1 0 1",
            "true false true",
            "length,name,prototype,isArray,from,of"
        ])
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

/// A long sparse range runs the plain spec loop over `has_index`/`get_index`, which take no
/// atom for a hole (M2-P1-INDEXOF-SCAN-REMOVED): indices on the prototype chain and a range
/// start past the end.
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

/// The spec loop finds integer keys past the array-index range: a key of 2^32 - 1 or more is
/// not an array index, so it is an ordinary string key (§10.1.11.1), stored after `length` or
/// any other key, that `has_index`/`get_index` find by its digits. A non-canonical key
/// ("04294967297") names no index; an own key shadows the prototype's; a getter in range
/// runs. Lengths reach 2^53 - 1.
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

/// Runs `call` on an isolate whose termination was requested before the script started, on
/// a thread with a 60 s wait (it only stops a hung test): the call must end terminated, and
/// the isolate must keep working. Nothing polls between the script's start and the loop of
/// `call`, so only a poll in that loop stops it.
fn assert_terminates(call: &'static str) {
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
            let r = iso.run_script(
                unit(&format!(
                    "var stage = 'before'; var r = {call}; stage = 'reached ' + r;"
                )),
                "call.js",
            );
            let terminated = r.is_err_and(|e| e.terminated);
            let stage = iso.global("stage").map(|v| iso.display(v));
            // The isolate keeps working.
            iso.run_script(unit("stage = 'again'"), "again.js")
                .expect("runs");
            let again = iso.global("stage").map(|v| iso.display(v));
            let _ = tx.send((terminated, stage, again));
        })
        .expect("a thread");
    let got = rx.recv_timeout(std::time::Duration::from_secs(60));
    assert_eq!(
        got,
        Ok((true, Some("before".to_string()), Some("again".to_string()))),
        "{call}"
    );
}

/// The step-10 loop of indexOf has no back edge and calls no JS on a hole, so it takes one
/// interrupt budget unit per index (`Cx::poll_interrupt`): termination stops a walk over
/// 2^53 - 1 holes, with or without a getter at index 0.
#[test]
fn termination_stops_index_of_over_a_long_hole_range() {
    for call in [
        "Array.prototype.indexOf.call({length: 2 ** 53 - 1}, 1)",
        "Array.prototype.indexOf.call({length: 2 ** 53 - 1, get 0() { return 0; }}, 1)",
    ] {
        assert_terminates(call);
    }
}

/// The M2-P4 loops over indices poll the same way (M2-P2-POLL): a walk over 2^53 - 1 holes or
/// missing elements with no JS call stops on termination.
#[test]
fn termination_stops_the_m2_p4_index_walks() {
    for call in [
        "Array.prototype.forEach.call({length: 2 ** 53 - 1}, function () {})",
        "Array.prototype.every.call({length: 2 ** 53 - 1}, function () { return false; })",
        "Array.prototype.lastIndexOf.call({length: 2 ** 53 - 1}, 1)",
        "Array.prototype.includes.call({length: 2 ** 53 - 1}, 1)",
        "Array.prototype.reverse.call({length: 2 ** 53 - 1})",
        "Array.prototype.sort.call({length: 2 ** 53 - 1})",
        "Array.prototype.copyWithin.call({length: 2 ** 53 - 1}, 1, 0)",
        "Array.prototype.reduce.call({length: 2 ** 53 - 1}, function () {}, 0)",
        "Array.prototype.find.call({length: 2 ** 53 - 1}, Math.max)",
        "Array.prototype.flat.call({length: 2 ** 53 - 1})",
    ] {
        assert_terminates(call);
    }
}

/// M2-P4 (B-ARRAY): names, lengths and attributes of the new methods, `get Array[@@species]`,
/// and the two Object statics of this module. (`Array.fromAsync` is month 3: async functions.)
#[test]
fn the_m2_p4_functions_are_installed() {
    assert_eq!(
        run_both(
            r#"var names = ['at','concat','copyWithin','every','fill','filter','find','findIndex','findLast','findLastIndex','flat','flatMap','forEach','includes','lastIndexOf','map','reduce','reduceRight','reverse','shift','slice','some','sort','splice','toLocaleString','toReversed','toSorted','toSpliced','unshift','with','keys','entries'];
             var out = [];
             for (var i = 0; i < names.length; i++) { var f = Array.prototype[names[i]]; out.push(f.name + '/' + f.length); }
             print(out.join(' '));
             print(Array.from.length, Array.of.length, Object.fromEntries.length, Object.groupBy.length, Object.groupBy.name);
             var d = Object.getOwnPropertyDescriptor(Array, Symbol.species);
             print(typeof d.get, d.set, d.enumerable, d.configurable, d.get.name, Array[Symbol.species] === Array);
             var m = Object.getOwnPropertyDescriptor(Array.prototype, 'map'); print(m.writable, m.enumerable, m.configurable);
             print(Object.getOwnPropertyNames(Array).join());
             print(Array.prototype.keys === Array.prototype.entries, Array.prototype.values === Array.prototype[Symbol.iterator]);"#
        ),
        lines(&[
            "at/1 concat/1 copyWithin/2 every/1 fill/1 filter/1 find/1 findIndex/1 findLast/1 findLastIndex/1 flat/0 flatMap/1 forEach/1 includes/1 lastIndexOf/1 map/1 reduce/1 reduceRight/1 reverse/0 shift/0 slice/2 some/1 sort/1 splice/2 toLocaleString/0 toReversed/0 toSorted/1 toSpliced/2 unshift/1 with/2 keys/0 entries/0",
            "1 0 1 2 groupBy",
            "function undefined false true get [Symbol.species] true",
            "true false true",
            "length,name,prototype,isArray,from,of",
            "false true",
        ])
    );
}

/// `forEach`, `map`, `filter`, `some`, `every`, `reduce`, `reduceRight`, `find*`, `includes`
/// and `lastIndexOf` (§23.1.3): holes are skipped (HasProperty) except by `find*` and
/// `includes`, `length` is read once, array-likes work, and a callback that is not callable is
/// a TypeError.
#[test]
fn callback_methods_skip_holes_and_read_array_likes() {
    assert_eq!(
        run_both(
            r#"var a = [1, , 3, 4];
             var seen = []; a.forEach(function (v, i, o) { seen.push(i + ':' + v + ':' + (o === a) + ':' + this.t); }, {t: 'T'}); print(seen.join());
             print(a.map(function (v) { return v * 2; }).length, 1 in a.map(function (v) { return v; }), a.filter(function (v) { return v > 1; }).join());
             print(a.some(function (v) { return v === undefined; }), a.every(function (v) { return v > 0; }), [].every(function () { return false; }), [].some(function () { return true; }));
             print(a.reduce(function (x, y) { return x + y; }), a.reduce(function (x, y) { return x + ',' + y; }, 's'), a.reduceRight(function (x, y) { return x + ',' + y; }));
             print([, , 5].reduce(function (x, y) { return x + y; }), [, , 5].reduceRight(function (x, y, i) { return x + y + i; }, 10));
             var al = {length: 3, 0: 'x', 2: 'z'};
             print(Array.prototype.map.call(al, function (v) { return v + '!'; }).join('|'), Array.prototype.filter.call(al, function () { return true; }).length);
             print(Array.prototype.find.call(al, function (v) { return v === undefined; }), Array.prototype.findIndex.call(al, function (v) { return v === undefined; }), Array.prototype.findLast.call(al, function (v) { return v; }), Array.prototype.findLastIndex.call(al, function (v) { return v === 'q'; }));
             print([1, 2, 3].includes(3, -1), [1, 2, 3].includes(1, 1), [, ].includes(undefined), [, ].indexOf(undefined), [0].includes(-0), ['a'].includes('a', Infinity));
             print([1, 2, 1, 2].lastIndexOf(2), [1, 2, 1, 2].lastIndexOf(2, 2), [1, 2].lastIndexOf(1, -3), [1, 2].lastIndexOf(2, undefined), [1, 2].lastIndexOf(1, -Infinity));
             var calls = 0; [1, 2, 3].forEach(function (v, i, o) { calls++; if (i === 0) { o.push(4); o.length = 2; } }); print(calls);
             var errs = [];
             ['forEach', 'map', 'filter', 'some', 'every', 'reduce', 'reduceRight', 'find', 'findIndex', 'findLast', 'findLastIndex', 'flatMap'].forEach(function (m) {
               try { [1][m]({}); errs.push(m + ':none'); } catch (e) { errs.push(e.constructor === TypeError ? 'T' : e.name); }
             });
             print(errs.join(''));
             try { [].reduce(function () {}); } catch (e) { print(e.constructor === TypeError); }
             try { [, , ].reduceRight(function () {}); } catch (e) { print(e.constructor === TypeError); }"#
        ),
        lines(&[
            "0:1:true:T,2:3:true:T,3:4:true:T",
            "4 false 3,4",
            "false true true false",
            "8 s,1,3,4 4,3,1",
            "5 17",
            "x!||z! 2",
            "undefined 1 z -1",
            "true false true -1 true false",
            "3 1 -1 -1 -1",
            "2",
            "TTTTTTTTTTTT",
            "true",
            "true",
        ])
    );
}

/// ArraySpeciesCreate (§10.4.2.3): `constructor[@@species]` makes the result of `map`,
/// `filter`, `slice`, `splice`, `concat`, `flat` and `flatMap`; null and undefined give an
/// Array, a non-constructor a TypeError; a non-array receiver always gets an Array.
/// IsConcatSpreadable (§23.1.3.2.1).
#[test]
fn array_species_create_and_is_concat_spreadable() {
    assert_eq!(
        run_both(
            r#"var a = [1, 2, 3];
             a.constructor = {};
             a.constructor[Symbol.species] = function (n) { this.args = arguments.length + ':' + n; };
             var r = a.map(function (x) { return x; }); print(r.args, r[2], Array.isArray(r), r.length);
             print(a.filter(function () { return true; }).args, a.slice(1).args, a.slice(1).length, a.splice(0, 1).args, a.concat().args, a.flat().args, a.flatMap(function (x) { return x; }).args);
             var b = [1]; b.constructor = {}; b.constructor[Symbol.species] = null; print(Array.isArray(b.map(function (x) { return x; })));
             var c = [1]; c.constructor = undefined; print(Array.isArray(c.slice()));
             var d = [1]; d.constructor = {}; d.constructor[Symbol.species] = 1;
             try { d.map(function () {}); } catch (e) { print(e.constructor === TypeError); }
             var e = [1]; e.constructor = 0; try { e.slice(); print('no throw'); } catch (x) { print('threw'); }
             var f = {length: 1, 0: 'q', constructor: function () { throw new Error('not used'); }};
             print(Array.isArray(Array.prototype.map.call(f, function (x) { return x; })));
             var spread = {length: 2, 0: 'a', 1: 'b'}; spread[Symbol.isConcatSpreadable] = true;
             var noSpread = [7, 8]; noSpread[Symbol.isConcatSpreadable] = false;
             var cc = [1].concat(spread, noSpread, 'str', [[2]], [, 3]);
             print(cc.length, cc[1], cc[2], cc[3] === noSpread, cc[4], Array.isArray(cc[5]), 7 in cc, cc[8]);"#
        ),
        lines(&[
            "1:3 3 false undefined",
            "1:0 1:2 2 1:1 1:0 1:0 1:0",
            "true",
            "true",
            "true",
            "threw",
            "true",
            "8 a b true str true true undefined",
        ])
    );
}

/// `sort` and `toSorted` (§23.1.3.30, §23.1.3.34): a stable sort, undefined values then holes
/// at the end, code-unit order without a comparator, the array unchanged when the comparator
/// throws, and the comparator check before ToObject.
#[test]
fn sort_is_stable_and_puts_undefined_and_holes_last() {
    assert_eq!(
        run_both(
            r#"print([5, 1, 4, 10, 2].sort().join(), [5, 1, 4, 10, 2].sort(function (x, y) { return x - y; }).join());
             var h = [3, undefined, , 1, , 2]; h.sort(); print(h.length, h.join('|'), 4 in h, 5 in h, 3 in h);
             var st = []; for (var i = 0; i < 100; i++) st.push({k: i % 3, i: i});
             st.sort(function (x, y) { return x.k - y.k; });
             var stable = true; for (var i = 1; i < 100; i++) { if (st[i - 1].k === st[i].k && st[i - 1].i > st[i].i) stable = false; } print(stable, st[0].i, st[99].i);
             var big = []; for (var i = 0; i < 2000; i++) big.push((i * 7919) % 2000);
             var copy = big.toSorted(function (x, y) { return y - x; }); var ok = copy.length === 2000; for (var i = 0; i < 2000; i++) if (copy[i] !== 1999 - i) ok = false; print(ok, big[1]);
             var t = [3, 1, 2]; try { t.sort(function () { throw new Error('cmp'); }); } catch (e) { print(e.message, t.join()); }
             print(['b', 'a', 'B', '\uD800', '￿', 'aa', ''].sort().map(function (s) { return s.length ? s.charCodeAt(0) : -1; }).join());
             var objs = [{toString: function () { return 'b'; }}, {toString: function () { return 'a'; }}]; print(objs.sort()[0].toString());
             var al = {length: 4, 0: 'd', 1: 'b', 3: 'a'}; Array.prototype.sort.call(al); print(al[0], al[1], al[2], 3 in al, al.length);
             print([, 1, , 0].toSorted().length, [, 1].toSorted()[1], 1 in [, 1].toSorted());
             try { [].sort(null); } catch (e) { print(e.constructor === TypeError); }
             try { [].toSorted('x'); } catch (e) { print(e.constructor === TypeError); }
             var nan = [3, 1, 2].sort(function () { return NaN; }); print(nan.join());
             var calls = 0; [1, 2].sort(function (x, y) { calls++; return x - y; }); print(calls > 0);"#
        ),
        lines(&[
            "1,10,2,4,5 1,2,4,5,10",
            "6 1|2|3||| false false true",
            "true 0 98",
            "true 1919",
            "cmp 3,1,2",
            "-1,66,97,97,98,55296,65535",
            "a",
            "a b d false 4",
            "4 undefined true",
            "true",
            "true",
            "3,1,2",
            "true",
        ])
    );
}

/// `sort` and `toSorted` without a comparator: SortCompare (§23.1.3.30.2 steps 5-6) calls
/// ToString on both values at every comparison, so an object's `toString` runs twice per
/// comparison of [x, x, x], and a throw from the 4th call ends the sort with the array
/// unchanged. Primitives sort as before. The number of comparisons is
/// implementation-defined (§23.1.3.30.1 step 4): V8 makes 2 for 3 values (node prints 4
/// calls), ep9's merge sort makes 3 (6 calls), so the script counts the comparisons of the
/// same sort with a comparator and prints whether the calls are twice that.
#[test]
fn sort_converts_both_values_at_each_comparison() {
    assert_eq!(
        run_both(
            r#"function counting() { var o = {n: 0, toString: function () { o.n++; return 'a'; }}; return o; }
             var cmp = 0; [1, 1, 1].sort(function () { cmp++; return 0; });
             var x = counting(); [x, x, x].sort(); print('sort', x.n === 2 * cmp, x.n >= 4);
             var y = counting(); [y, y, y].toSorted(); print('toSorted', y.n === 2 * cmp, y.n >= 4);
             var z = counting(); [z, 'b', 1, z].sort(); print('mixed', z.n > 2);
             function fourth() { var o = {n: 0, toString: function () { o.n++; if (o.n === 4) throw new Error('fourth'); return 'a'; }}; return o; }
             var f = fourth(); var arr = [f, f, f]; try { arr.sort(); print('sort no throw'); } catch (e) { print('sort', e.message, f.n, arr[0] === f); }
             var g = fourth(); try { [g, g, g].toSorted(); print('toSorted no throw'); } catch (e) { print('toSorted', e.message, g.n); }
             print([3, 'b', 1, true, null, 'a'].sort().join());
             try { [Symbol(), 1].sort(); } catch (e) { print(e.constructor === TypeError); }"#
        ),
        lines(&[
            "sort true true",
            "toSorted true true",
            "mixed true",
            "sort fourth 4 true",
            "toSorted fourth 4",
            "1,3,a,b,,true",
            "true",
        ])
    );
}

/// The index arithmetic of `slice`, `splice`, `toSpliced`, `copyWithin`, `fill`, `reverse`,
/// `shift`, `unshift`, `at`, `toReversed` and `with`, on arrays with holes and on array-likes,
/// with the 2^53 - 1 length checks.
#[test]
fn slice_splice_copy_within_fill_reverse_shift_unshift_at_with() {
    assert_eq!(
        run_both(
            r#"var a = [1, 2, 3, 4, 5];
             print(a.slice(-2).join(), a.slice(1, -1).join(), a.slice(10).length, a.slice(-Infinity, Infinity).length, a.slice(2, 1).length);
             var s = [1, 2, 3, 4, 5]; print(s.splice(-2).join(), s.join(), s.splice(1, 0, 'a', 'b').length, s.join(), s.splice().length, s.splice(0, 1, 9).join(), s.join());
             var s2 = [1, 2, 3]; print(s2.splice(1, Infinity).join(), s2.join(), s2.splice(0, -1, 'x').length, s2.join());
             print([1, 2, 3].toSpliced(1).join(), [1, 2, 3].toSpliced().join(), [1, 2, 3].toSpliced(-1, 1, 'p', 'q').join(), [, 1].toSpliced(0, 0).hasOwnProperty(0));
             print([1, 2, 3, 4, 5].copyWithin(1, 0).join(), [1, 2, 3, 4, 5].copyWithin(0, 2, 4).join(), [1, , 3].copyWithin(0, 1).hasOwnProperty(0), [1, 2, 3, 4, 5].copyWithin(-2, -4, -3).join());
             print([1, 2, 3].fill(7).join(), [1, 2, 3].fill(7, -1).join(), [1, 2, 3].fill(7, 1, 2).join(), Array.prototype.fill.call({length: 2}, 'f')[1]);
             var r = [1, , 3, 4]; r.reverse(); print(r.join('|'), 2 in r, 1 in r);
             var al = {length: 3, 0: 'a', 2: 'c'}; Array.prototype.reverse.call(al); print(al[0], 1 in al, al[2]);
             var sh = [1, 2, 3]; print(sh.shift(), sh.join(), [].shift(), Array.prototype.shift.call({length: 0}));
             var o = {length: 2, 0: 'x', 1: 'y'}; print(Array.prototype.shift.call(o), o.length, o[0], 1 in o);
             var u = [3]; print(u.unshift(1, 2), u.join(), u.unshift(), Array.prototype.unshift.call({length: 1, 0: 'b'}, 'a'));
             print([1, 2, 3].at(0), [1, 2, 3].at(-4), [1, 2, 3].at(1.7), [1, 2, 3].toReversed().join(), [, 1].toReversed().hasOwnProperty(1));
             print([1, 2, 3].with(0, 'z').join(), [1, 2, 3].with(-3, 'y').join());
             try { [1].with(1, 0); } catch (e) { print(e.constructor === RangeError); }
             try { [1].with(-2, 0); } catch (e) { print(e.constructor === RangeError); }
             try { Array.prototype.unshift.call({length: Math.pow(2, 53) - 1}, 1); } catch (e) { print(e.constructor === TypeError); }
             try { Array.prototype.splice.call({length: Math.pow(2, 53) - 1}, 0, 0, 1); } catch (e) { print(e.constructor === TypeError); }
             print(Array.prototype.unshift.call({length: Math.pow(2, 53) - 1}));"#
        ),
        lines(&[
            "4,5 2,3,4 0 5 0",
            "4,5 1,2,3 0 1,a,b,2,3 0 1 9,a,b,2,3",
            "2,3 1 0 x,1",
            "1 1,2,3 1,2,p,q true",
            "1,1,2,3,4 3,4,3,4,5 false 1,2,3,2,5",
            "7,7,7 1,2,7 1,7,3 f",
            "4|3||1 false true",
            "c false a",
            "1 2,3 undefined undefined",
            "x 1 y false",
            "3 1,2,3 3 2",
            "1 undefined 2 3,2,1 true",
            "z,2,3 y,2,3",
            "true",
            "true",
            "true",
            "true",
            "9007199254740991",
        ])
    );
}

/// `flat` and `flatMap` (FlattenIntoArray, §23.1.3.13.1): depth conversion, holes, array-likes
/// (only Arrays are flattened), a mapper with `thisArg`, and 100 nested levels.
#[test]
fn flat_and_flat_map() {
    assert_eq!(
        run_both(
            r#"var n = [1, [2, [3, [4, [5]]]]];
             print(n.flat().length, n.flat(2).length, n.flat(Infinity).join(), n.flat(0).length, n.flat(-1).length, n.flat('1').length);
             print([1, , 3, [4, , 6]].flat().join('|'), [1, , 3, [4, , 6]].flat().length);
             print([1, 2].flatMap(function (x, i, o) { return [x, i, o.length]; }).join(), [1, 2].flatMap(function (x) { return [[x]]; })[0].length, [1].flatMap(function () { return this.v; }, {v: 'tv'}).join());
             var al = {length: 2, 0: [1], 1: 2}; print(Array.prototype.flat.call(al).join());
             var notArr = {length: 1, 0: 'x'}; print([notArr].flat()[0] === notArr);
             var deep = []; var cur = deep; for (var i = 0; i < 100; i++) { var nx = []; cur.push(nx); cur = nx; } cur.push('end'); print(deep.flat(Infinity).join());"#
        ),
        lines(&[
            "3 4 1,2,3,4,5 2 2 3",
            "1|3|4|6 4",
            "1,0,2,2,1,2 1 tv",
            "1,2",
            "true",
            "end",
        ])
    );
}

/// `Array.from` (§23.1.2.1) over an iterable (closed when the mapper throws) and an array-like,
/// with a constructor `this`; `Array.of` (§23.1.2.3).
#[test]
fn array_from_and_array_of() {
    assert_eq!(
        run_both(
            r#"print(Array.from('a😀b').length, Array.from([1, 2], function (v, i) { return v + i + this.d; }, {d: 10}).join(), Array.from({length: 2, 0: 'p'}).join('|'), Array.from({length: 0}).length);
             var it = {}; it[Symbol.iterator] = function () { var i = 0; return {next: function () { i++; return {done: i > 3, value: i}; }, return: function () { print('closed'); return {}; }}; };
             print(Array.from(it).join(), Array.from(it, function (v) { return v * v; }).join());
             try { Array.from(it, function (v) { if (v === 2) throw new Error('map'); return v; }); } catch (e) { print(e.message); }
             function C() { this.made = arguments.length + ':' + arguments[0]; }
             var c1 = Array.from.call(C, [1, 2]); print(c1.made, c1.length, c1[1], c1 instanceof C);
             var c2 = Array.from.call(C, {length: 2, 0: 'a'}); print(c2.made, c2.length, c2[0]);
             var c3 = Array.from.call({}, [1]); print(Array.isArray(c3));
             print(Array.of().length, Array.of(3).length, Array.of(3)[0], Array.of.call(C, 'x', 'y').made, Array.of.call(C, 'x').length, Array.isArray(Array.of.call(undefined, 1)));
             try { Array.from([], 'no'); } catch (e) { print(e.constructor === TypeError); }
             try { Array.from(null); } catch (e) { print(e.constructor === TypeError); }
             var bad = {}; bad[Symbol.iterator] = 1; try { Array.from(bad); } catch (e) { print(e.constructor === TypeError); }
             var nulled = {length: 1, 0: 'n'}; nulled[Symbol.iterator] = null; print(Array.from(nulled).join());"#
        ),
        lines(&[
            "3 11,13 p| 0",
            "1,2,3 1,4,9",
            "closed",
            "map",
            "0:undefined 2 2 true",
            "1:2 2 a",
            "true",
            "0 1 3 1:2 1 true",
            "true",
            "true",
            "true",
            "n",
        ])
    );
}

/// `keys` and `entries` (VM builtins 0.28 and 0.29): array iterators of kind key and key+value
/// (§23.1.5.1). A key step reads no element; the length is read at every step; for-of and
/// spread take the generic protocol.
#[test]
fn keys_and_entries_iterators() {
    assert_eq!(
        run_both(
            r#"var a = ['x', 'y'];
             var k = a.keys(), e = a.entries();
             print(k.next === [][Symbol.iterator]().next, k[Symbol.iterator]() === k, Object.prototype.toString.call(e));
             var r = k.next(); print(r.value, r.done, k.next().value, k.next().done, k.next().value);
             r = e.next(); print(Array.isArray(r.value), r.value.length, r.value[0], r.value[1], e.next().value.join(), e.next().done);
             var out = []; for (var p of a.entries()) out.push(p[0] + '=' + p[1]); print(out.join());
             var ks = []; for (var i of [, , 'c'].keys()) ks.push(i); print(ks.join());
             print([...'ab'].join(), [...['m', 'n'].keys()].join(), [...['m', 'n'].entries()].join('|'));
             var al = {length: 2, 0: 'g'}; print([...Array.prototype.entries.call(al)].join('|'), [...Array.prototype.keys.call(al)].join());
             var grow = [1]; var gi = grow.keys(); gi.next(); grow.push(2); print(gi.next().value, gi.next().done, (grow.push(3), gi.next().done));
             var getterHit = 0; var g = {length: 1, get 0() { getterHit++; return 'v'; }};
             var gk = Array.prototype.keys.call(g); gk.next(); print(getterHit); Array.prototype.entries.call(g).next(); print(getterHit);
             try { Array.prototype.keys.call(null); } catch (x) { print(x.constructor === TypeError); }
             var str = Array.prototype.entries.call('hi'); print(str.next().value.join());"#
        ),
        lines(&[
            "true true [object Array Iterator]",
            "0 false 1 true undefined",
            "true 2 0 x 1,y true",
            "0=x,1=y",
            "0,1,2",
            "a,b 0,1 0,m|1,n",
            "0,g|1, 0,1",
            "1 true true",
            "0",
            "1",
            "true",
            "0,h",
        ])
    );
}

/// `Array.prototype.push` on any object (§23.1.3.23): ToObject, LengthOfArrayLike, `[[Set]]`
/// for each item and for `length`, and the TypeError past 2^53 - 1.
#[test]
fn push_is_generic() {
    assert_eq!(
        run_both(
            r#"var o = {length: 1, 0: 'a'}; print(Array.prototype.push.call(o, 'b', 'c'), o.length, o[2]);
             var e = {}; print(Array.prototype.push.call(e), e.length);
             var w = {length: '2'}; print(Array.prototype.push.call(w, 'x'), w[2], typeof w.length);
             var log = []; var g = {get length() { log.push('get'); return 3; }, set length(v) { log.push('set ' + v); }};
             print(Array.prototype.push.call(g, 1), log.join());
             try { Array.prototype.push.call({length: Math.pow(2, 53) - 1}, 1); } catch (x) { print(x.constructor === TypeError); }
             print(Array.prototype.push.call({length: Math.pow(2, 53) - 1}));
             try { Array.prototype.push.call('ab', 'c'); } catch (x) { print(x.constructor === TypeError); }
             var n = Array.prototype.push.call(5, 1); print(n);
             try { Array.prototype.push.call(undefined); } catch (x) { print(x.constructor === TypeError); }"#
        ),
        lines(&[
            "3 3 c",
            "0 0",
            "3 x number",
            "4 get,set 4",
            "true",
            "9007199254740991",
            "true",
            "1",
            "true",
        ])
    );
}

/// `Object.fromEntries` (§20.1.2.7) and `Object.groupBy` (§20.1.2.12; defined by this module on
/// %Object%): groups in first-appearance order on a null-prototype object, and the iterator
/// closed when the callback throws.
#[test]
fn object_from_entries_and_group_by() {
    assert_eq!(
        run_both(
            r#"var o = Object.fromEntries([['a', 1], ['b', 2], ['a', 3]]); print(Object.keys(o).join(), o.a, o.b);
             var it = {}; var closed = 0; it[Symbol.iterator] = function () { return {next: function () { return {done: false, value: 7}; }, return: function () { closed++; return {}; }}; };
             try { Object.fromEntries(it); } catch (e) { print(e.constructor === TypeError, closed); }
             try { Object.fromEntries(null); } catch (e) { print(e.constructor === TypeError); }
             var g = Object.groupBy([1, 2, 3, 4, 5, 6], function (v, i) { return v % 3 === 0 ? 'three' : i < 2 ? 'low' : 'rest'; });
             print(Object.keys(g).join(), g.low.join(), g.rest.join(), g.three.join(), g.toString, Array.isArray(g.low));
             var n = Object.groupBy('abca', function (c) { return c; }); print(Object.keys(n).join(), n.a.length);
             var num = Object.groupBy([1, 2, 3], function (v) { return v > 1 ? 2 : 1; }); print(Object.keys(num).join(), num[2].join());
             try { Object.groupBy([1], null); } catch (e) { print(e.constructor === TypeError); }
             try { Object.groupBy(null, function () {}); } catch (e) { print(e.constructor === TypeError); }
             var c2 = 0; var it2 = {}; it2[Symbol.iterator] = function () { return {next: function () { return {done: false, value: 1}; }, return: function () { c2++; return {}; }}; };
             try { Object.groupBy(it2, function () { throw new Error('cb'); }); } catch (e) { print(e.message, c2); }"#
        ),
        lines(&[
            "a,b 3 2",
            "true 1",
            "true",
            "low,three,rest 1,2 4,5 3,6 undefined true",
            "a,b,c 2",
            "1,2 2,3",
            "true",
            "true",
            "cb 1",
        ])
    );
}

/// `toLocaleString` (§23.1.3.32): Invoke(element, "toLocaleString") for each element that is
/// not undefined or null, joined with "," (as V8).
#[test]
fn to_locale_string_invokes_each_element() {
    assert_eq!(
        run_both(
            r#"var x = {toLocaleString: function () { return 'X' + (this === x); }};
             print([x, null, undefined, x].toLocaleString(), [].toLocaleString() === '', [{toLocaleString: function () { return 5; }}].toLocaleString());
             try { [{toLocaleString: 1}].toLocaleString(); } catch (e) { print(e.constructor === TypeError); }
             print(Array.prototype.toLocaleString.call({length: 2, 0: x}));"#
        ),
        lines(&["Xtrue,,,Xtrue true 5", "true", "Xtrue,",])
    );
}
