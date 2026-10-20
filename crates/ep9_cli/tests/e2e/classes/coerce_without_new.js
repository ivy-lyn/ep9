var ran = 0;
class C { constructor() { ran = ran + 1; return 7; } }
class D extends C { constructor() { super(); ran = ran + 100; } }
function check(label, f) {
  try { print(label, f()); } catch (e) { print(label, e instanceof TypeError, e.message); }
}
check("add", function () { return { valueOf: C } + 1; });
check("template", function () { return `${{ toString: C }}`; });
check("mul", function () { return { valueOf: D } * 2; });
check("compare", function () { return { valueOf: C } > 0; });
check("key", function () { var o = {}; return o[{ toString: C }]; });
check("join", function () { return [{ toString: C }].join(); });
print("ran", ran);
print(new C() instanceof C, new D() instanceof C, ran);
