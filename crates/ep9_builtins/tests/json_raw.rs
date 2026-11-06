//! json-parse-with-source (M6-P1 SPEC-B): `JSON.rawJSON`, `JSON.isRawJSON`, the raw JSON step
//! of JSON.stringify, and the reviver's `context` argument with `source`.
//!
//! The expected lines are node v26.13.0's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier), so the
//! parse records (values and keys on a ScratchList, indices in Rust) survive every
//! collection.

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `%JSON%` with `rawJSON` and `isRawJSON` (V8's key order, lengths, attributes, not
/// constructors), and the raw JSON object: null prototype, frozen, one enumerable key, the
/// class test (a proxy or an object that inherits from one is not raw), and the errors
/// (the edge whitespace, an object or array outside, a bad JSON text, a Symbol).
#[test]
fn the_json_object_and_raw_json_objects() {
    assert_eq!(
        common::run_both(
            r##"
print(Reflect.ownKeys(JSON).map(String).join());
print(JSON.rawJSON.length, JSON.rawJSON.name, JSON.isRawJSON.length, JSON.isRawJSON.name);
var d = Object.getOwnPropertyDescriptor(JSON, 'rawJSON');
print(d.writable, d.enumerable, d.configurable, typeof d.value);
d = Object.getOwnPropertyDescriptor(JSON, 'isRawJSON');
print(d.writable, d.enumerable, d.configurable, typeof d.value);
print(Object.getPrototypeOf(JSON.rawJSON) === Function.prototype, JSON.rawJSON.hasOwnProperty('prototype'));
try { new JSON.rawJSON('1'); } catch (e) { print(e.name); }
try { new JSON.isRawJSON(1); } catch (e) { print(e.name); }
var r = JSON.rawJSON('"a\\u0041"');
print(Object.getPrototypeOf(r), Object.isFrozen(r), Object.isExtensible(r), Object.keys(r).join(), r.rawJSON);
var rd = Object.getOwnPropertyDescriptor(r, 'rawJSON');
print(rd.value, rd.writable, rd.enumerable, rd.configurable);
print(Object.prototype.toString.call(r), typeof r, JSON.isRawJSON(r));
print(JSON.isRawJSON(), JSON.isRawJSON({ rawJSON: '1' }), JSON.isRawJSON(Object.create(r)), JSON.isRawJSON(new Proxy(r, {})));
print([1, -0.5e3, 'null', 'true', 'false', '"x"', 9007199254740993].map(function (v) { return JSON.rawJSON(v).rawJSON; }).join('|'));
function err(f) { try { f(); return 'none'; } catch (e) { return e.name; } }
print(['', ' 1', '1 ', '\t1', '1\n', '\r1', '{}', '[]', '[1]', '{"a":1}', '1 2', 'tru', '01', '"a', 'undefined', '1,'].map(function (t) { return err(function () { JSON.rawJSON(t); }); }).join());
print(err(function () { JSON.rawJSON(Symbol('1')); }), err(function () { JSON.rawJSON(); }), err(function () { JSON.rawJSON({}); }), err(function () { JSON.rawJSON([]); }));
print(err(function () { 'use strict'; r.rawJSON = 'x'; }), err(function () { 'use strict'; r.y = 1; }), r.rawJSON);
var o = { toString: function () { return '42'; } };
print(JSON.rawJSON(o).rawJSON, JSON.rawJSON(true).rawJSON, JSON.rawJSON(null).rawJSON);
var p = JSON.rawJSON(false); var q = { a: 'hi' }; Object.setPrototypeOf(q, p); print(q.rawJSON, JSON.isRawJSON(q), JSON.stringify(q));
"##
        ),
        lines(&[
            "parse,stringify,rawJSON,isRawJSON,Symbol(Symbol.toStringTag)",
            "1 rawJSON 1 isRawJSON",
            "true false true function",
            "true false true function",
            "true false",
            "TypeError",
            "TypeError",
            "null true false rawJSON \"a\\u0041\"",
            "\"a\\u0041\" false true false",
            "[object Object] object true",
            "false false false false",
            "1|-500|null|true|false|\"x\"|9007199254740992",
            "SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError",
            "TypeError SyntaxError SyntaxError SyntaxError",
            "TypeError TypeError \"a\\u0041\"",
            "42 true null",
            "false false {\"a\":\"hi\"}",
        ])
    );
}

/// JSON.stringify of raw JSON objects: as values, elements and members, with a gap, from a
/// replacer or a toJSON, at the top level; a proxy of one or an object that inherits from
/// one is an ordinary object.
#[test]
fn stringify_writes_raw_json_as_it_is() {
    assert_eq!(
        common::run_both(
            r##"
var R = JSON.rawJSON;
print(JSON.stringify(R('1e1000')), JSON.stringify(R('"\\u0041"')), JSON.stringify(R('-0')), JSON.stringify(R('1.50')));
print(JSON.stringify({ a: R('1'), b: [R('null'), R('true'), R('"s"')], c: { d: R('-2.5e-3') } }));
print(JSON.stringify([R('1'), undefined, function () {}, R('2')]));
print(JSON.stringify({ a: R('1'), b: [R('2')] }, null, 2).split('\n').join('|'));
print(JSON.stringify({ a: 1, b: 'x' }, function (k, v) { return typeof v === 'number' ? R(String(v * 10)) : v; }));
print(JSON.stringify({ toJSON: function () { return R('7'); } }), JSON.stringify([{ toJSON: function () { return R('8'); } }]));
print(JSON.stringify(R('3'), function (k, v) { return v; }));
print(JSON.stringify(R('3'), function (k, v) { return k === '' ? v : 0; }));
var seen = []; JSON.stringify({ x: R('1') }, function (k, v) { seen.push(k + ':' + JSON.isRawJSON(v)); return v; }); print(seen.join());
print(JSON.stringify(new Proxy(R('5'), {})), JSON.stringify(Object.create(R('6'))));
print(JSON.stringify({ a: R('1') }, ['a']), JSON.stringify(R('9'), ['rawJSON']));
var log = []; var raw = R('11');
print(JSON.stringify([raw, raw, { k: raw }]));
"##
        ),
        lines(&[
            "1e1000 \"\\u0041\" -0 1.50",
            "{\"a\":1,\"b\":[null,true,\"s\"],\"c\":{\"d\":-2.5e-3}}",
            "[1,null,null,2]",
            "{|  \"a\": 1,|  \"b\": [|    2|  ]|}",
            "{\"a\":10,\"b\":\"x\"}",
            "7 [8]",
            "3",
            "3",
            ":false,x:true",
            "{\"rawJSON\":\"5\"} {}",
            "{\"a\":1} 9",
            "[11,11,{\"k\":11}]",
        ])
    );
}

/// The reviver's third argument: a new plain object per call, with an own data property
/// `source` (the exact text: escapes, exponents and trailing zeros kept) for a primitive whose
/// value is still the parsed one, and none for an array, an object or a changed value; a
/// duplicate key gives its last value's text; an added or replaced container has no
/// records.
#[test]
fn parse_reviver_gets_the_source() {
    assert_eq!(
        common::run_both(
            r##"
function show(text) {
  var out = [];
  var r = JSON.parse(text, function (k, v, ctx) {
    out.push(k + '=' + (Object.getPrototypeOf(ctx) === Object.prototype) + ':' + Object.keys(ctx).join('/') + ':' + ctx.source + ':' + arguments.length);
    return v;
  });
  print(out.join(' '));
  return r;
}
show('1'); show(' -1.50e+2 '); show('"a\\u0041\\n"'); show('true'); show('null'); show('[]'); show('{}');
show('[1, "2", [true, null], {"x": 3.0}]');
show('{"b": 2, "1": 1, "a": {"c": [0]}, "b": 4}');
show('{"__proto__": 5, "x": 1e1}');
var ctxs = []; JSON.parse('[1,2]', function (k, v, c) { ctxs.push(c); return v; }); print(ctxs[0] !== ctxs[1], ctxs.length);
var d; JSON.parse('5', function (k, v, c) { d = Object.getOwnPropertyDescriptor(c, 'source'); return v; }); print(d.value, d.writable, d.enumerable, d.configurable);
var log = [];
JSON.parse('{"p": 1, "q": 2, "r": [3, 4]}', function (k, v, c) {
  log.push(k + ':' + c.source);
  if (k === 'p') { this.q = 2; this.r = [3, 5]; }
  return v;
});
print(log.join(' '));
log = [];
JSON.parse('[1, "x", 1]', function (k, v, c) {
  log.push(k + ':' + c.source);
  if (k === '0') { this[1] = 'x'; this[2] = 1.0; }
  return v;
});
print(log.join(' '));
log = [];
JSON.parse('{"a": {"b": 1}, "c": 2}', function (k, v, c) {
  log.push(k + ':' + c.source);
  if (k === 'b') { delete this.b; }
  if (k === 'a') { return undefined; }
  return v;
});
print(log.join(' '));
log = [];
var r = JSON.parse('[[1], 2]', function (k, v, c) { log.push(k + ':' + c.source); if (k === '0' && Array.isArray(v)) { return 'R'; } return v; });
print(log.join(' '), JSON.stringify(r));
log = [];
JSON.parse('{"a": 1, "b": 2}', function (k, v, c) { log.push(k + ':' + c.source); if (k === 'a') { this.c = 3; } return v; });
print(log.join(' '));
print(JSON.parse('[1,2]', function (k, v) { return v; }).join(), JSON.parse('{"a":[1]}', function (k, v) { return Array.isArray(v) ? v.length : v; }).a);
"##
        ),
        lines(&[
            "=true:source:1:3",
            "=true:source:-1.50e+2:3",
            "=true:source:\"a\\u0041\\n\":3",
            "=true:source:true:3",
            "=true:source:null:3",
            "=true::undefined:3",
            "=true::undefined:3",
            "0=true:source:1:3 1=true:source:\"2\":3 0=true:source:true:3 1=true:source:null:3 2=true::undefined:3 x=true:source:3.0:3 3=true::undefined:3 =true::undefined:3",
            "1=true:source:1:3 b=true:source:4:3 0=true:source:0:3 c=true::undefined:3 a=true::undefined:3 =true::undefined:3",
            "__proto__=true:source:5:3 x=true:source:1e1:3 =true::undefined:3",
            "true 3",
            "5 true true true",
            "p:1 q:2 0:undefined 1:undefined r:undefined :undefined",
            "0:1 1:\"x\" 2:1 :undefined",
            "b:1 a:undefined c:2 :undefined",
            "0:1 0:undefined 1:2 :undefined [\"R\",2]",
            "a:1 b:2 :undefined",
            "1,2 1",
        ])
    );
}

/// The records of an object with more than 16 entries (integer keys in falling order,
/// duplicates: the hashed lookup), of a long array, and of a deep array.
#[test]
fn parse_reviver_records_of_long_and_deep_values() {
    assert_eq!(
        common::run_both(
            r##"
var parts = [];
for (var i = 30; i > 0; i--) parts.push('"' + i + '": ' + i + '.0');
for (var j = 0; j < 30; j++) parts.push('"k' + j + '": "v' + j + '"');
for (var j = 0; j < 30; j += 3) parts.push('"k' + j + '": ' + j + 'e0');
parts.push('"5": 55');
var srcs = {}, n = 0, bad = 0;
JSON.parse('{' + parts.join(', ') + '}', function (k, v, c) { if (k !== '') { n++; srcs[k] = c.source; } return v; });
for (var i = 1; i <= 30; i++) if (srcs[i] !== (i === 5 ? '55' : i + '.0')) bad++;
for (var j = 0; j < 30; j++) if (srcs['k' + j] !== (j % 3 === 0 ? j + 'e0' : '"v' + j + '"')) bad++;
print(n, bad);
var small = []; JSON.parse('{"a": 1, "b": 2, "a": 3.0}', function (k, v, c) { small.push(k + ':' + c.source); return v; }); print(small.join(' '));
var a = []; for (var i = 0; i < 100; i++) a.push(i % 7 === 0 ? '"s' + i + '"' : String(i) + (i % 2 ? '.5' : ''));
var got = []; JSON.parse('[' + a.join(',') + ']', function (k, v, c) { if (k !== '') got.push(c.source); return v; });
print(got.length, got.join(',') === a.join(','));
var deep = ''; for (var i = 0; i < 100; i++) deep += '['; deep += '1'; for (var i = 0; i < 100; i++) deep += ']';
var ds = []; JSON.parse(deep, function (k, v, c) { if (typeof v === 'number') ds.push(c.source); return v; }); print(ds.join());
"##
        ),
        lines(&["60 0", "a:3.0 b:2 :undefined", "100 true", "1",])
    );
}

/// A long object with a reviver (no GC stress; 3,900 keys: 3,000 integer keys in falling
/// order, then string keys with 450 duplicates): every source is right, and the lookup is
/// not quadratic in the key count.
#[test]
fn parse_reviver_on_a_long_object() {
    let src = r##"
var parts = [];
for (var i = 3000; i > 0; i--) parts.push('"' + i + '": ' + i + '.0');
for (var j = 0; j < 900; j++) parts.push('"k' + j + '": ' + j);
for (var j = 0; j < 900; j += 2) parts.push('"k' + j + '": "d' + j + '"');
var n = 0, bad = 0;
JSON.parse('{' + parts.join(',') + '}', function (k, v, c) {
  if (k === '') return v;
  n++;
  var want = typeof v === 'string' ? '"' + v + '"' : (k.charAt(0) === 'k' ? String(v) : v + '.0');
  if (c.source !== want) bad++;
  return v;
});
print(n, bad);
"##;
    assert_eq!(
        common::on_big_stack(move || common::run(src, false)),
        lines(&["3900 0"])
    );
}
