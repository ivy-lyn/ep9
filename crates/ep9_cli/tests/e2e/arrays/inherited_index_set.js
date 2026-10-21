// A store to a missing array element walks the prototype chain (OrdinarySet): an inherited
// setter runs, a getter without a setter or a read-only property blocks it. An own element
// is stored in place. Each case removes its prototype property again.
var calls = 0, seen;
function setter(v) { calls++; seen = v; }

Object.defineProperty(Object.prototype, "0", { set: setter, configurable: true });
var a = [];
a[0] = 7;
print("setter, number key:", calls, seen, a.length, a.hasOwnProperty("0"));
var b = [];
b["0"] = 8;
print("setter, string key:", calls, seen, b.length, b.hasOwnProperty("0"));
var c = [1];
c[0] = 9;
print("own element:", calls, c[0], c.length);
a[2] = 3;
print("other index:", calls, a.length, a.hasOwnProperty("2"), a[2]);
var r = b.push(5, 6);
print("push:", r, calls, seen, b.length, b.hasOwnProperty("0"), b[1]);
delete Object.prototype[0];

Object.defineProperty(Array.prototype, "1", { set: setter, configurable: true });
var h = [1, , 3];
h[1] = 4;
print("hole:", calls, seen, h.length, h.hasOwnProperty("1"));
delete Array.prototype[1];

Object.defineProperty(Array.prototype, "0", { get: function () { return "g"; }, configurable: true });
var g = [];
g[0] = 1;
print("getter, sloppy:", g.length, g.hasOwnProperty("0"), g[0]);
(function () {
  "use strict";
  var t = [];
  try { t[0] = 1; print("getter, strict: stored"); } catch (e) { print("getter, strict:", e.name, t.length); }
  try { t["0"] = 1; print("getter, strict: stored"); } catch (e) { print("getter, strict:", e.name, t.length); }
})();
delete Array.prototype[0];

Object.defineProperty(Object.prototype, "0", { value: "ro", writable: false, configurable: true });
var w = [];
w[0] = 1;
print("read-only, sloppy:", w.length, w.hasOwnProperty("0"), w[0]);
(function () {
  "use strict";
  var t = [];
  try { t[0] = 1; print("read-only, strict: stored"); } catch (e) { print("read-only, strict:", e.name, t.length); }
  try { t.push(1); print("read-only, push: stored"); } catch (e) { print("read-only, push:", e.name, t.length); }
})();
delete Object.prototype[0];

// Without the prototype properties, stores append again.
var d = [];
for (var i = 0; i < 100; i++) d[i] = i;
d.push(100);
print("after:", d.length, d[0], d[99], d[100], d.hasOwnProperty("0"));
