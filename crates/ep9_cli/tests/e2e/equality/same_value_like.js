function same(a, b) { if (a === b) return a !== 0 || 1 / a === 1 / b; return a !== a && b !== b; }
print(same(0, -0), same(-0, -0), same(NaN, NaN), same(1, 1), same("a", "a"), same({}, {}));
