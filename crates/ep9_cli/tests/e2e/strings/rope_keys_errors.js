// A rope as a property key, a global name, an error message and a template part.
var name = "dynamically" + "BuiltGlobal";
globalThis[name] = 7;
print(dynamicallyBuiltGlobal, typeof globalThis["dynamically" + "BuiltGlobal"]);
try {
  throw new TypeError("message " + "built from " + "parts " + 3);
} catch (e) {
  print(e.message, String(e));
}
var parts = [];
for (var i = 0; i < 50; i++) parts.push("item" + i);
var joined = parts.join(", ");
print(joined.length, joined.slice(-8), `${joined}`.length);
var x = { toString() { return "an object as a long string"; } };
print(x + "!", "<" + x + ">");
print(["b".repeat(20) + "a", "b".repeat(20)].sort().join("|"));
var long = "";
for (var j = 0; j < 100; j++) long += String.fromCharCode(65 + (j % 26));
print(long.split("Z").length, long.replace("ABC", "-").slice(0, 10), eval("'" + long + "'") === long);
