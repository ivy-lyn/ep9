function tag(strings, ...values) {
  var out = "";
  for (var i = 0; i < strings.length; i++) {
    out += "[" + strings[i] + "]";
    if (i < values.length) out += "<" + values[i] + ">";
  }
  return out;
}
print(tag`a${1}b${2}c`);
print(tag`${"x"}`);
function raw(s) { return s.raw[0] + "|" + s[0]; }
print(raw`a\nb`);
function same(s) { return s; }
function get() { return same`x`; }
print(get() === get());
