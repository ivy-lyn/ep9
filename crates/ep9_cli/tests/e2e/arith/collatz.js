function steps(n) { var c = 0; while (n !== 1) { n = n % 2 ? 3 * n + 1 : n / 2; c++; } return c; }
var best = 0, arg = 0;
for (var i = 1; i < 1000; i++) { var s = steps(i); if (s > best) { best = s; arg = i; } }
print(arg, best);
