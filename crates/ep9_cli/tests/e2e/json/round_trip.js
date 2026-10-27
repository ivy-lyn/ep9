// JSON.parse and JSON.stringify on a small document: key order, nesting, escapes, numbers,
// the gap, and a parse of the stringified text.
var text = '{"name": "ep9", "tags": ["js", "\\u00e9", "\\ud83d\\ude00"], "n": [0, -0, 1.5e3, 1e21, 0.1],' +
  ' "nested": {"a": {"b": {"c": [true, false, null]}}}, "2": "two", "1": "one"}';
var doc = JSON.parse(text);
print(Object.keys(doc).join());
print(doc.tags[1].charCodeAt(0), doc.tags[2].length, 1 / doc.n[1], doc.nested.a.b.c.length);
var out = JSON.stringify(doc);
print(out);
print(JSON.stringify(JSON.parse(out)) === out);
print(JSON.stringify(doc.nested, null, 2));
print(JSON.stringify({ d: undefined, f: print, s: 'a \u0007' }));
