var x = 1;
function f() { try { return x; } finally { x = 2; } }
print(f(), x);
var o = { v: 1 };
function g() { try { return o; } finally { o.v = 99; } }
print(g().v);
