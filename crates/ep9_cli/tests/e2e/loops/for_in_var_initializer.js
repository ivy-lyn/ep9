// Annex B.3.5 `for (var x = init in o)`: the initializer's own scopes (a class) are children of
// the for scope. A debug build used to panic ("scopes are entered in tree order").
for (var a = class {} in {}) {}
print(a.name);
for (var b = class named {} in {}) {}
print(b.name);
for (var c = class { static m() { return typeof c; } } in {}) {}
print(c.name, c.m());
function f() {
  let y = 2;
  for (var x = class { static h = () => y; } in {}) {}
  return x.h();
}
print(f());
function g() {
  for (var x = class C { static h = () => C; } in {}) {}
  return x.h() === x;
}
print(g());
function h() {
  var r = [];
  for (var x = class { static [(r.push(1), "k")] = 1; } in (r.push(2), {})) {}
  return r.join() + " " + x.k;
}
print(h());
function k() {
  var y = 7;
  for (var x = eval("var z = 3; y + z") in { p: 1 }) print(x);
  return [x, z].join();
}
print(k());
for (var d = async function () {} in {}) {}
for (var e = function* () {} in {}) {}
print(d.name, e.name);
