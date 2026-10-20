print(5 & 3, 5 | 3, 5 ^ 3, ~5, ~-1, ~0);
print(-5 & 3, -5 | 3, -5 ^ 3, ~2147483647);
print(0xff & 0x0f, 0xf0 | 0x0f, 0xffffffff & 1, 0xffffffff | 0);
print(1.9 | 0, -1.9 | 0, NaN | 0, Infinity | 0, "12" | 0, "x" | 0);
