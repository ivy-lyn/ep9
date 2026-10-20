function f() { var s = 0; for (var i = 0; i < arguments.length; i++) s += arguments[i]; return arguments.length + ":" + s; }
print(f(), f(1), f(1, 2, 3));
function g(a, b) { return arguments.length + " " + a + " " + b + " " + arguments[2]; }
print(g(1), g(1, 2, 3));
