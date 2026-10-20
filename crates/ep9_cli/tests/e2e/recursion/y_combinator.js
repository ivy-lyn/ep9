var Y = function (f) { return (function (x) { return f(function (v) { return x(x)(v); }); })(function (x) { return f(function (v) { return x(x)(v); }); }); };
var fact = Y(function (self) { return function (n) { return n === 0 ? 1 : n * self(n - 1); }; });
print(fact(10));
