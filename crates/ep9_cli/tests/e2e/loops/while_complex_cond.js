var a = 0, b = 10, steps = 0;
while (a < b && (b - a) % 7 !== 0 || steps < 2) { a++; b--; steps++; }
print(a, b, steps);
