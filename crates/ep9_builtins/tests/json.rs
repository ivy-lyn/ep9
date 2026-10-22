//! JSON (§25.5): `JSON.parse` (the grammar, the reviver walk), `JSON.stringify` (replacer
//! function and array, gap, toJSON, wrapper objects, the cycle check, well-formed output), the
//! nesting limits, `%JSON%` itself, and Number::toString (the digits stringify prints) against
//! a node golden.
//!
//! The expected lines are node v24.22.1's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier).

mod common;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{Cx, Isolate, IsolateConfig, JsResult, Local};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `%JSON%`: its keys, attributes, the functions' length and name, @@toStringTag.
#[test]
fn the_json_object() {
    assert_eq!(
        common::run_both(
            r#"
            print(Reflect.ownKeys(JSON).filter(function (k) { return k !== 'rawJSON' && k !== 'isRawJSON'; }).map(String).join());
            print(Object.getPrototypeOf(JSON) === Object.prototype, typeof JSON, Object.prototype.toString.call(JSON));
            var d = Object.getOwnPropertyDescriptor(this, 'JSON');
            print(d.writable, d.enumerable, d.configurable);
            var t = Object.getOwnPropertyDescriptor(JSON, Symbol.toStringTag);
            print(t.value, t.writable, t.enumerable, t.configurable);
            print(JSON.parse.length, JSON.parse.name, JSON.stringify.length, JSON.stringify.name);
            var p = Object.getOwnPropertyDescriptor(JSON, 'parse');
            print(p.writable, p.enumerable, p.configurable);
            try { new JSON.parse('1'); } catch (e) { print(e.name); }
            "#
        ),
        lines(&[
            "parse,stringify,Symbol(Symbol.toStringTag)",
            "true object [object JSON]",
            "true false true",
            "JSON false false true",
            "2 parse 3 stringify",
            "true false true",
            "TypeError",
        ])
    );
}

/// The JSON grammar: values, whitespace, escapes, numbers, and the SyntaxErrors.
#[test]
fn parse_grammar() {
    assert_eq!(
        common::run_both(
            r#"
            var o = JSON.parse(' {"a" : [1, -2.5e3, 0.25, 1E2, -0, true, false, null], "b": {"c": "d"}, "": 1} ');
            print(o.a.join('|'), 1 / o.a[4], o.b.c, o[''], Object.keys(o).join());
            print(JSON.parse('"\\u0041\\u00e9\\ud83d\\ude00\\"\\\\\\/\\b\\f\\n\\r\\t"').split('').map(function (c) { return c.charCodeAt(0); }).join());
            print(JSON.parse('"\ud800"').charCodeAt(0), JSON.parse('"\\uD800"').length);
            print(JSON.parse('123456789012345678901234567890'), JSON.parse('1e400'), JSON.parse('-1e-400') === 0, JSON.parse('9007199254740993'));
            print(JSON.parse('[]').length, Object.keys(JSON.parse('{}')).length, JSON.parse('\t\r\n 7 \n'));
            var bad = ['', ' ', '{', '[1,]', '{"a":1,}', '01', '-', '1.', '.5', '+1', '1e', '"\t"', "'a'", '[1] 2',
                       'tru', 'nul', 'NaN', 'Infinity', '{a:1}', '"\\x41"', '"\\u12"', '/*c*/1', '1//', '\u00a01', '"abc'];
            print(bad.map(function (s) { try { JSON.parse(s); return 'ok'; } catch (e) { return e.name; } }).join());
            print(JSON.parse(1), JSON.parse(true), JSON.parse(null), JSON.parse({ toString: function () { return '[3]'; } })[0]);
            "#
        ),
        lines(&[
            "1|-2500|0.25|100|0|true|false| -Infinity d 1 a,b,",
            "65,233,55357,56832,34,92,47,8,12,10,13,9",
            "55296 1",
            "1.2345678901234568e+29 Infinity true 9007199254740992",
            "0 0 7",
            "SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError",
            "1 true null 3",
        ])
    );
}

/// `__proto__` is an own data property, a duplicate key keeps the last value and its first
/// position, and no setter on a prototype runs (CreateDataProperty).
#[test]
fn parse_defines_own_properties() {
    assert_eq!(
        common::run_both(
            r#"
            var o = JSON.parse('{"__proto__": [1], "a": 1, "b": 2, "a": 3}');
            print(Object.getPrototypeOf(o) === Object.prototype, Array.isArray(o.__proto__), Object.keys(o).join(), o.a);
            var hits = 0;
            Object.defineProperty(Object.prototype, 'x', { set: function () { hits++; }, configurable: true });
            Object.defineProperty(Array.prototype, '0', { set: function () { hits++; }, configurable: true });
            var p = JSON.parse('{"x": 1, "y": [5, 6]}');
            print(hits, p.x, p.y[0], p.y.length, Object.getOwnPropertyNames(p.y).join());
            delete Object.prototype.x; delete Array.prototype[0];
            "#
        ),
        lines(&["true true __proto__,a,b 3", "0 1 5 2 0,1,length",])
    );
}

/// InternalizeJSONProperty: the walk order, `this`, deletion on undefined, the keys taken
/// before the walk, array lengths read once.
#[test]
fn parse_reviver() {
    assert_eq!(
        common::run_both(
            r#"
            var log = [];
            var r = JSON.parse('{"a": [1, {"b": 2}], "c": 3, "d": 4}', function (k, v) {
              log.push(k + ':' + (typeof v === 'object' ? (Array.isArray(v) ? 'arr' : 'obj') : v));
              if (k === 'c') return undefined;
              if (typeof v === 'number') return v * 10;
              return v;
            });
            print(log.join(' '));
            print(JSON.stringify(r), 'c' in r);
            var holder;
            JSON.parse('[7]', function (k, v) { if (k === '0') holder = this; return v; });
            print(Array.isArray(holder), holder[0]);
            var seen = [];
            JSON.parse('{"p": 1, "q": 2}', function (k, v) { seen.push(k); if (k === 'p') { delete this.q; this.z = 9; } return v; });
            print(seen.join());
            print(JSON.parse('[1, 2, 3]', function (k, v) { if (k === '0') this.length = 1; return v; }).length);
            print(JSON.parse('5', 42), JSON.parse('5', function (k, v) { return [k, v, this[k]]; }).join('/'));
            "#
        ),
        lines(&[
            "0:1 b:2 1:obj a:arr c:3 d:4 :obj",
            "{\"a\":[10,{\"b\":20}],\"d\":40} false",
            "true 7",
            "p,q,",
            "1",
            "5 /5/5",
        ])
    );
}

/// JSON.stringify of each kind of value, and the wrapper objects.
#[test]
fn stringify_values() {
    assert_eq!(
        common::run_both(
            r#"
            print(JSON.stringify(null), JSON.stringify(true), JSON.stringify('a"b\\'), JSON.stringify(-0), JSON.stringify(1e21), JSON.stringify(NaN), JSON.stringify(-Infinity));
            print(JSON.stringify(undefined), JSON.stringify(function () {}), JSON.stringify(Symbol('s')));
            print(JSON.stringify({ u: undefined, f: function () {}, s: Symbol(), n: null, x: 1 }), JSON.stringify([undefined, function () {}, Symbol()]));
            print(JSON.stringify(new Number(1.5)), JSON.stringify(new String('w')), JSON.stringify(new Boolean(false)), JSON.stringify(Object(Symbol())));
            var n = new Number(3); n.valueOf = function () { return 4; }; n.toString = function () { return '5'; };
            var s = new String('a'); s.toString = function () { return 'b'; };
            print(JSON.stringify([n, s]));
            var k = { b: 1, a: 2, 2: 3, 1: 4 }; Object.defineProperty(k, 'h', { value: 5 }); k[Symbol()] = 6;
            print(JSON.stringify(k));
            print(JSON.stringify([[], {}, [[1]], { a: {} }]), JSON.stringify([,1]));
            var a = [1, 2]; a.x = 3;
            print(JSON.stringify(a));
            "#
        ),
        lines(&[
            "null true \"a\\\"b\\\\\" 0 1e+21 null null",
            "undefined undefined undefined",
            "{\"n\":null,\"x\":1} [null,null,null]",
            "1.5 \"w\" false {}",
            "[4,\"b\"]",
            "{\"1\":4,\"2\":3,\"b\":1,\"a\":2}",
            "[[],{},[[1]],{\"a\":{}}] [null,1]",
            "[1,2]",
        ])
    );
}

/// QuoteJSONString: the short escapes, other control characters, lone surrogates
/// (well-formed-json-stringify) and surrogate pairs.
#[test]
fn stringify_quotes_strings() {
    assert_eq!(
        common::run_both(
            r#"
            print(JSON.stringify('\b\t\n\f\r"\\/\u0000\u001f\u007f\u2028'));
            print(JSON.stringify('\ud800'), JSON.stringify('\udc00\ud800'), JSON.stringify('\ud83d\ude00').length, JSON.stringify('x\udbff'));
            var o = {}; o['\ud800k\n'] = 1;
            print(JSON.stringify(o));
            "#
        ),
        lines(&[
            "\"\\b\\t\\n\\f\\r\\\"\\\\/\\u0000\\u001f\u{7f}\u{2028}\"",
            "\"\\ud800\" \"\\udc00\\ud800\" 4 \"x\\udbff\"",
            "{\"\\ud800k\\n\":1}",
        ])
    );
}

/// toJSON and the replacer function: arguments, `this`, order, and the value they return.
#[test]
fn stringify_to_json_and_replacer_function() {
    assert_eq!(
        common::run_both(
            r#"
            var log = [];
            var o = { a: { toJSON: function (k) { log.push('toJSON:' + k); return [k]; } }, b: [7] };
            print(JSON.stringify(o, function (k, v) {
              log.push(k + ':' + (this === o ? 'o' : Array.isArray(this) ? 'arr' : typeof this));
              return typeof v === 'number' ? v + 1 : v;
            }));
            print(log.join(' '));
            print(JSON.stringify({ toJSON: function () { return undefined; } }), JSON.stringify({ toJSON: 1, x: 2 }));
            Number.prototype.toJSON = function () { return 'N' + this; };
            print(JSON.stringify([1, { a: 2 }]));
            delete Number.prototype.toJSON;
            print(JSON.stringify(1, function (k, v) { return k === '' ? { x: undefined } : v; }), JSON.stringify(1, function (k, v) { return k === '' ? [v, v] : 0; }));
            "#
        ),
        lines(&[
            "{\"a\":[\"a\"],\"b\":[8]}",
            ":object toJSON:a a:o 0:arr b:o 0:arr",
            "undefined {\"toJSON\":1,\"x\":2}",
            "[1,{\"a\":2}]",
            "{} [0,0]",
        ])
    );
}

/// The replacer array: strings, numbers and their wrappers, duplicates and other values
/// dropped, the list order, nested objects (the list applies at every level), arrays not
/// filtered.
#[test]
fn stringify_replacer_array() {
    assert_eq!(
        common::run_both(
            r#"
            var o = { a: 1, b: { a: 2, c: 3 }, 1: [4, { a: 5, z: 6 }], c: 7 };
            print(JSON.stringify(o, ['c', 'a', 1, 'a', new String('b'), new Number(1), {}, true, null, Symbol()]));
            var n = new Number(9); n.toString = function () { return 'b'; };
            print(JSON.stringify(o, [n]), JSON.stringify(o, []), JSON.stringify([1, 2], ['0']));
            var r = ['a']; r[2] = 'c';
            print(JSON.stringify(o, r));
            "#
        ),
        lines(&[
            "{\"c\":7,\"a\":1,\"1\":[4,{\"a\":5}],\"b\":{\"c\":3,\"a\":2}}",
            "{\"b\":{}} {} [1,2]",
            "{\"a\":1,\"c\":7}",
        ])
    );
}

/// The gap: a number (clamped to 10), a string (cut to 10 code units), their wrappers, and the
/// layout of nested and empty containers.
#[test]
fn stringify_gap() {
    // `show` prints a multi-line text on one line, with "|" for each line feed.
    assert_eq!(
        common::run_both(
            r#"
            function show(t) { return t === undefined ? t : t.split('\n').join('|'); }
            var v = { a: [1, { b: [] }, {}], c: 'x' };
            print(show(JSON.stringify(v, null, 2)));
            print(JSON.stringify([1], null, 20) === JSON.stringify([1], null, 10), show(JSON.stringify([1], null, 1.9)), JSON.stringify([1], null, -3));
            print(show(JSON.stringify([1, [2]], null, '-=-=-=-=-=-=-')));
            print(show(JSON.stringify({ a: 1 }, null, new Number(1))), show(JSON.stringify({ a: 1 }, null, new String('\t'))), JSON.stringify({ a: 1 }, null, true));
            print(show(JSON.stringify({ u: undefined, a: 1, f: function () {} }, null, 1)));
            "#
        ),
        lines(&[
            "{|  \"a\": [|    1,|    {|      \"b\": []|    },|    {}|  ],|  \"c\": \"x\"|}",
            "true [| 1|] [1]",
            "[|-=-=-=-=-=1,|-=-=-=-=-=[|-=-=-=-=-=-=-=-=-=-=2|-=-=-=-=-=]|]",
            "{| \"a\": 1|} {|\t\"a\": 1|} {\"a\":1}",
            "{| \"a\": 1|}",
        ])
    );
}

/// The cycle check (a TypeError, then the next call works), a value seen twice without a
/// cycle, and errors from getters, toJSON and the replacer.
#[test]
fn stringify_cycles_and_abrupt_completions() {
    assert_eq!(
        common::run_both(
            r#"
            var a = []; a[0] = { x: a };
            try { JSON.stringify(a); } catch (e) { print(e.name); }
            var c = {}; c.self = c;
            try { JSON.stringify(c, null, 2); } catch (e) { print(e.name); }
            var shared = { s: 1 };
            print(JSON.stringify([shared, shared, { t: shared }]));
            var g = { get boom() { throw new RangeError('g'); } };
            try { JSON.stringify(g); } catch (e) { print(e.name, e.message); }
            try { JSON.stringify({ toJSON: function () { throw new EvalError('t'); } }); } catch (e) { print(e.name); }
            try { JSON.stringify([1], function () { throw new URIError('r'); }); } catch (e) { print(e.name); }
            try { JSON.stringify(1, null, { valueOf: function () { throw 1; } }); } catch (e) { print('no throw'); }
            print(JSON.stringify(1, null, { valueOf: function () { throw 1; } }));
            "#
        ),
        lines(&[
            "TypeError",
            "TypeError",
            "[{\"s\":1},{\"s\":1},{\"t\":{\"s\":1}}]",
            "RangeError g",
            "EvalError",
            "URIError",
            "1",
        ])
    );
}

/// A new isolate with `ep9_builtins::TABLE`, a collection before every `stress`-th
/// allocation (0: none) and the heap verifier after each one.
fn isolate_with_stress(stress: u32) -> Isolate {
    Isolate::new(IsolateConfig {
        heap: HeapConfig {
            gc_stress: stress,
            verify_after_gc: stress != 0,
            ..HeapConfig::default()
        },
        builtins: Some(&ep9_builtins::TABLE),
        ..IsolateConfig::default()
    })
    .expect("an isolate")
}

/// Runs `src` and gives the global `result` as Rust text.
fn run_for_result(iso: &mut Isolate, src: &str) -> String {
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("compiles");
    iso.run_script(unit, "deep.js").expect("runs");
    iso.with_cx(|cx| {
        let global = cx.global();
        let v = cx.get(global, "result")?;
        cx.to_rust_string(v)
    })
    .expect("a result")
}

/// JSON.parse of 1,000,000 nested arrays: no Rust recursion, so the default stack is
/// enough; under GC stress (a collection every 131,072nd allocation, then the verifier: about
/// 8 collections over 1,000,000 live arrays, each one in the middle of the parse) the open
/// arrays stay alive. The reviver walk and JSON.stringify of the same nesting throw the
/// RangeError (the depth cap or the native stack check), and the isolate goes on.
#[test]
fn deep_nesting() {
    let mut iso = isolate_with_stress(1 << 17);
    let src = r#"
        var n = 1000000;
        var text = '['.repeat(n) + ']'.repeat(n);
        var a = JSON.parse(text);
        var depth = 1, x = a;
        while (x.length) { x = x[0]; depth++; }
        var r1, r2;
        try { JSON.stringify(a); r1 = 'no throw'; } catch (e) { r1 = e.name + ': ' + e.message; }
        try { JSON.parse(text, function (k, v) { return v; }); r2 = 'no throw'; } catch (e) { r2 = e.name; }
        var result = [depth, r1, r2, JSON.stringify(JSON.parse('[[1]]'))].join();
    "#;
    assert_eq!(
        run_for_result(&mut iso, src),
        "1000000,RangeError: Maximum call stack size exceeded,RangeError,[[1]]"
    );
    iso.collect_garbage();
    iso.verify_heap().expect("a valid heap");
}

/// JSON.stringify of a 100,000-deep array (made by JSON.parse) throws RangeError, not a crash,
/// with and without GC stress (a collection every 10,000th allocation, then the verifier: one
/// before every allocation would make the 100,000 arrays quadratic). Without stress, also the
/// cap itself: 10,000 levels work on a big native stack, and one more level fails.
#[test]
fn stringify_deep_array_throws_range_error() {
    const NEST: &str = "function nest(n) { return JSON.parse('['.repeat(n) + ']'.repeat(n)); }";
    let deep = "var r = []; \
        try { JSON.stringify(nest(100000)); r.push('no throw'); } catch (e) { r.push(e.name); }";
    let cap = "r.push(JSON.stringify(nest(10000)).length); \
        try { JSON.stringify(nest(10001)); r.push('no throw'); } catch (e) { r.push(e.name); }";
    for (stress, src, want) in [
        (
            0,
            format!("{NEST} {deep} {cap} var result = r.join();"),
            "RangeError,20000,RangeError",
        ),
        (
            10_000,
            format!("{NEST} {deep} var result = r.join();"),
            "RangeError",
        ),
    ] {
        let got = common::on_big_stack(move || {
            let mut iso = isolate_with_stress(stress);
            let r = run_for_result(&mut iso, &src);
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            r
        });
        assert_eq!(got, want, "gc_stress {stress}");
    }
}

/// A stringify and parse round trip of a larger object graph under GC stress.
#[test]
fn round_trip_under_gc_stress() {
    assert_eq!(
        common::run_both(
            r#"
            var v = [];
            for (var i = 0; i < 20; i++) v.push({ i: i, s: 'v' + i, a: [i, i / 4, null, true], o: { k: 'é\u2028' + i } });
            var t = JSON.stringify(v);
            var back = JSON.parse(t);
            print(t.length, JSON.stringify(back) === t, back[19].o.k, JSON.stringify(back, null, 1).length);
            "#
        ),
        lines(&["1141 true é\u{2028}19 2082"])
    );
}

// ---- Number::toString against node ---------------------------------------------------------

/// The golden written by `crates/ep9_cli/tests/e2e/json/number_tostring_golden.mjs` (node).
fn golden_text() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ep9_cli/tests/e2e/json/number_tostring.golden");
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .replace("\r\n", "\n")
}

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

/// 2^e for 0 <= e < 64, exactly (from the bits: clippy.toml bans `powi`, H36).
fn pow2(e: u64) -> f64 {
    f64::from_bits((1023 + e) << 52)
}

/// Value `i` of the golden (the script's `goldenDouble`).
fn golden_double(g: &mut XorShift, i: u64) -> f64 {
    let r = g.next();
    let s = g.next();
    match i % 4 {
        0 => f64::from_bits(r),
        1 => {
            let biased = 1023 - 80 + (s % 140);
            f64::from_bits((r & (1 << 63)) | (biased << 52) | (r & ((1 << 52) - 1)))
        }
        2 => {
            let q = r >> (11 + (s % 40));
            let t = (s >> 8) % 27;
            q as f64 / pow2(t)
        }
        _ => {
            let q = r >> (11 + (s % 53));
            q as f64 * pow2((s >> 8) % 16)
        }
    }
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

/// ToString of a number through the builtin ABI (`Cx::to_string`, numconv
/// `number_to_string`).
fn to_string(cx: &mut Cx<'_>, x: f64) -> JsResult<String> {
    cx.to_rust_string(Local::number(x))
}

/// Number::toString of 100,000 random doubles (1,000 blocks of 100, each pinned by a hash
/// and its last string) and of 200 doubles exactly halfway between two shortest digit
/// strings (§6.1.6.1.20 step 5: the even one) equals node's.
#[test]
fn number_to_string_matches_the_node_golden() {
    let golden = golden_text();
    let mut iso = isolate_with_stress(0);
    let mut g = XorShift(0x9e37_79b9_7f4a_7c15);
    let (mut blocks, mut ties, mut bad) = (0, 0, Vec::new());
    for line in golden.lines() {
        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        match parts[..] {
            ["block", b, hash, last] => {
                let b: u64 = b.parse().expect("a block number");
                let strings: Vec<String> = iso
                    .with_cx(|cx| {
                        let mut v = Vec::new();
                        for i in 0..100 {
                            let x = golden_double(&mut g, b * 100 + i);
                            v.push(to_string(cx, x)?);
                        }
                        Ok(v)
                    })
                    .expect("ToString");
                let got = format!("{:08x}", fnv(&strings));
                if got != hash || strings[99] != last {
                    bad.push(format!(
                        "block {b}: hash {got} (node {hash}), last {} (node {last}); \
                         node's block: number_tostring_golden.mjs --block {b}",
                        strings[99]
                    ));
                }
                blocks += 1;
            }
            ["tie", bits, want] => {
                let x = f64::from_bits(u64::from_str_radix(bits, 16).expect("hex bits"));
                let got = iso.with_cx(|cx| to_string(cx, x)).expect("ToString");
                if got != want {
                    bad.push(format!("tie {bits}: {got} (node {want})"));
                }
                ties += 1;
            }
            _ => panic!("bad golden line {line:?}"),
        }
    }
    assert_eq!((blocks, ties), (1000, 200), "the golden's size");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The known tie of the week-4 carry-over (R5) and its JSON form.
#[test]
fn number_to_string_tie_goes_to_even() {
    assert_eq!(
        common::run_both(
            "print(900719925474099.25, JSON.stringify([900719925474099.25, -0.18521499633789062]), 0.5, 2.5e-7);"
        ),
        lines(&["900719925474099.2 [900719925474099.2,-0.18521499633789062] 0.5 2.5e-7"])
    );
}
