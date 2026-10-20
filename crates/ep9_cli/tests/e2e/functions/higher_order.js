function map(xs, f) { var r = []; for (var i = 0; i < xs.length; i++) r.push(f(xs[i], i)); return r; }
function filter(xs, f) { var r = []; for (var i = 0; i < xs.length; i++) if (f(xs[i])) r.push(xs[i]); return r; }
function reduce(xs, f, acc) { for (var i = 0; i < xs.length; i++) acc = f(acc, xs[i]); return acc; }
var xs = [1, 2, 3, 4, 5, 6];
print(map(xs, (x, i) => x * i).join(","), filter(xs, (x) => x % 2 === 0).join(","), reduce(xs, (a, b) => a + b, 0));
