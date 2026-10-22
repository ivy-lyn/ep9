// A reviver that drops and maps values, and a replacer function and array.
var log = [];
var v = JSON.parse('{"a": 1, "b": [2, 3], "c": {"d": 4}}', function (k, x) {
  log.push(k === '' ? '(root)' : k);
  if (k === 'd') return undefined;
  return typeof x === 'number' ? x * 2 : x;
});
print(log.join(' '));
print(JSON.stringify(v));
print(JSON.stringify(v, function (k, x) { return k === 'a' ? 'A' : x; }));
print(JSON.stringify(v, ['b', 'a']));
print(JSON.stringify({ date: { toJSON: function (key) { return 'when:' + key; } } }));
var cyclic = { x: 1 };
cyclic.me = cyclic;
try {
  JSON.stringify(cyclic);
} catch (e) {
  print(e.name);
}
try {
  JSON.parse('{"a": 1,}');
} catch (e) {
  print(e.name);
}
