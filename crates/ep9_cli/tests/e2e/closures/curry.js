function curry3(f) { return function (a) { return function (b) { return function (c) { return f(a, b, c); }; }; }; }
var vol = curry3(function (a, b, c) { return a * b * c; });
print(vol(2)(3)(4));
var compose = function (f, g) { return function (x) { return f(g(x)); }; };
var inc = function (x) { return x + 1; }, dbl = function (x) { return x * 2; };
print(compose(inc, dbl)(5), compose(dbl, inc)(5));
