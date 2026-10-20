// `(a?.b)()` and `(a?.b)`x``: the parenthesized optional chain is still a reference, so the
// call gets `a` as `this`. A nullish base makes the callee undefined: the call throws.
var o = {
  x: 7,
  m: function () { "use strict"; return this === o; },
  g: function () { return this.x; },
  t: function (s) { "use strict"; return this === o && s[0]; },
};
print((o?.m)());
print((o?.g)());
print((o?.["m"])());
var p = { q: o };
print((p?.q.m)());
print((p.q?.m)());
print(((o?.m))());
print((o?.t)`tag`);
print((o?.m)?.());
var n = null, u;
try { (n?.m)(); } catch (e) { print("null", e.constructor === TypeError); }
try { (u?.m)`x`; } catch (e) { print("undefined tag", e.constructor === TypeError); }
print((n?.m)?.());
// The arguments run before the call throws.
var calls = 0;
try { (u?.m)(calls++); } catch (e) { print("args", e.constructor === TypeError, calls); }
// Not a reference: the comma operator gives a plain value.
print((0, o?.g)());
