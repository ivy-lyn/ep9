var f = function fact(n) { return n <= 1 ? 1 : n * fact(n - 1); };
print(f(6), typeof fact);
var g = function self() { return typeof self; };
print(g());
