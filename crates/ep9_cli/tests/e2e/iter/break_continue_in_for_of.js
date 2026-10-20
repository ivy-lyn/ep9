var out = [];
outer: for (var row of [[1, 2, 3], [4, 5, 6], [7, 8, 9]]) {
  for (var v of row) {
    if (v === 2) continue;
    if (v === 5) continue outer;
    if (v === 8) break outer;
    out.push(v);
  }
}
print(out.join(","));
function firstEven(xs) { for (const x of xs) if (x % 2 === 0) return x; return -1; }
print(firstEven([1, 3, 6, 8]), firstEven([1]));
