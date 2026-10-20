function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
var out = [];
for (var i = 0; i <= 20; i++) out.push(fib(i));
print(out.join(","));
