var sq = x => x * x;
var add = (a, b) => a + b;
var obj = () => ({ k: 1 });
var block = (n) => { var r = 1; for (var i = 2; i <= n; i++) r *= i; return r; };
print(sq(7), add(2, 3), obj().k, block(5), (() => "iife")());
