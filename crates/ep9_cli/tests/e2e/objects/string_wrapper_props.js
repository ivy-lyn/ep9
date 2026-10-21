// The own properties of a String wrapper: `length` and the indices below it are read through
// [[Get]], and they are neither writable nor configurable.
var w = Object("ab");
print(typeof w, w.length, w[0], w[1], w[2], w["1"]);
w.length = 5;
w[0] = "z";
w[7] = "q";
print(w.length, w[0], w[7]);
print(delete w[0], delete w.length, delete w[7], w[0], w.length, w[7]);
print("0" in w, "2" in w, "length" in w, "7" in w);
(function () {
  "use strict";
  try { w.length = 1; print("no throw"); } catch (e) { print(e.name); }
  try { w[1] = "q"; print("no throw"); } catch (e) { print(e.name); }
  try { delete w[1]; print("no throw"); } catch (e) { print(e.name); }
  print(w.length, w[1]);
})();
print("ab".length, "ab"[1], "ab"[2], "ab"["0"]);
