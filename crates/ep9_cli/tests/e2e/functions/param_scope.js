var x = "outer";
function f(a = x) { var x = "inner"; return a + "/" + x; }
print(f());
function g(a, b = () => a) { a = 2; return b(); }
print(g(1));
