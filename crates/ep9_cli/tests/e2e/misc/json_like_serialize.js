function ser(v) {
  if (v === null) return "null";
  if (typeof v === "string") return '"' + v + '"';
  if (typeof v !== "object") return "" + v;
  if ("length" in v) { var a = []; for (var i = 0; i < v.length; i++) a.push(ser(v[i])); return "[" + a.join(",") + "]"; }
  var parts = [];
  for (var k in v) parts.push('"' + k + '":' + ser(v[k]));
  return "{" + parts.join(",") + "}";
}
print(ser({ a: 1, b: [true, null, "s"], c: { d: -0.5 } }));
