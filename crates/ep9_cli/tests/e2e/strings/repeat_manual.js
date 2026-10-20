function repeat(s, n) { var r = ""; while (n > 0) { if (n & 1) r += s; s += s; n >>= 1; } return r; }
print(repeat("ab", 5), repeat("x", 0), repeat("-", 16).length);
