function memo(f) { var cache = {}; var hits = 0; var m = function (n) { if (n in cache) { hits++; return cache[n]; } var v = f(n); cache[n] = v; return v; }; m.hits = function () { return hits; }; return m; }
var fib = memo(function (n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); });
print(fib(50), fib.hits());
