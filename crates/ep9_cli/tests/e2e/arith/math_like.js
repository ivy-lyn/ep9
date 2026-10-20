function abs(x) { return x < 0 ? -x : x; }
function floor(x) { var i = x | 0; return i > x ? i - 1 : i; }
function gcd(a, b) { while (b) { var t = b; b = a % b; a = t; } return a; }
function isqrt(n) { var x = n; var y = (x + 1) >> 1; while (y < x) { x = y; y = (x + (n / x | 0)) >> 1; } return x; }
print(abs(-3), abs(3), floor(2.7), floor(-2.7), gcd(48, 18), isqrt(1000000), isqrt(99));
