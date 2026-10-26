// Ropes (CONS) from += and from prepends, read by every kind of reader.
var s = "";
for (var i = 0; i < 300; i++) s += "ab" + (i % 10);
var t = "";
for (var j = 299; j >= 0; j--) t = "ab" + (j % 10) + t;
print(s.length, s === t, s == t, s < t + "x", t > s.slice(0, 50));
print(s[0], s[1], s[2], s[899], s.charAt(450), s.charCodeAt(451));
print(s.indexOf("ab9ab0"), s.lastIndexOf("ab5"), s.substring(30, 45));
var k = "key-" + "with-a-long-name";
var o = {};
o[k] = 1;
o["key-with-a-long-name"] += 1;
print(o[k], Object.keys(o)[0] === k, k in o);
var m = new Map();
m.set(k, "m");
print(m.get("key-with-" + "a-long-name"));
print(JSON.stringify({ [k]: k + "!" }));
print(Symbol.for(k) === Symbol.for("key-with-a-long-name"));
switch (k) {
  case "key-with-a-long-name":
    print("switch ok");
    break;
  default:
    print("switch bad");
}
print(Number("1234567" + "890123"), +("  42" + "          "), parseInt("12345678" + "9012"));
var w = "€" + "uro sign and more text";
print(w.length, w.charCodeAt(0), (w + w).toUpperCase().slice(0, 5));
