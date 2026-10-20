function adder(n) { return function (x) { return x + n; }; }
var add5 = adder(5), add10 = adder(10);
print(add5(1), add10(1), adder(-1)(1));
function setter(v) { return function (nv) { var old = v; v = nv; return old; }; }
var s = setter(1);
print(s(2), s(3), s(4));
