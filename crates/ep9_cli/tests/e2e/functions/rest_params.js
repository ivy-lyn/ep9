function f(first, ...rest) { return first + ":" + rest.length + ":" + rest.join("|"); }
print(f(1), f(1, 2), f(1, 2, 3, 4));
var g = (...all) => all.length;
print(g(), g(1, 2, 3));
