function f(a, b = 2, c = a + b) { return [a, b, c].join(","); }
print(f(1), f(1, 5), f(1, undefined, 0), f(1, null));
var calls = 0;
function g(x = ++calls) { return x; }
g(); g(5); g();
print(calls);
