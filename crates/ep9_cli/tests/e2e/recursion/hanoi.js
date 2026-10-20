var moves = [];
function hanoi(n, a, b, c) { if (n === 0) return 0; var k = hanoi(n - 1, a, c, b); if (moves.length < 7) moves.push(a + c); return k + 1 + hanoi(n - 1, b, a, c); }
print(hanoi(10, "A", "B", "C"), moves.join(" "));
