// Reflect.set with an Array receiver whose `length` is not writable (a DICTIONARY array since
// M5-P2 ELEM2): §10.1.9.2 step 2.c.iii returns false before any conversion of the value
// (Codex M5-P2 r1 elem2 2).
var r = [1, 2];
Object.defineProperty(r, "length", { writable: false });
var calls = 0;
var v = { valueOf: function () { calls++; return 0; } };
print("receiver", Reflect.set({}, "length", 0, r), Reflect.set({}, "length", v, r), calls, r.length);
print("receiver, same value", Reflect.set({}, "length", 2, r), calls, r.length);
print("own", Reflect.set(r, "length", 0), Reflect.set(r, "length", v), calls, r.length);
// A writable length still converts once per ToUint32 and ToNumber (ArraySetLength).
var w = [1, 2, 3];
print("writable", Reflect.set({}, "length", v, w), calls, w.length);
try {
  Reflect.set({}, "length", -1, w);
  print("no throw");
} catch (e) {
  print("writable, bad length", e.constructor.name, w.length);
}
// Strict code: a failed [[Set]] throws a TypeError.
(function () {
  "use strict";
  var o = Object.create(r);
  try {
    o.length = 5;
    print("no throw", o.length);
  } catch (e) {
    print("strict", e.constructor.name, calls, Object.getOwnPropertyNames(o).length);
  }
})();
