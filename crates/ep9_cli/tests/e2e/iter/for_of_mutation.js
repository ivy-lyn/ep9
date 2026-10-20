var a = [1, 2, 3];
var seen = [];
for (var x of a) { seen.push(x); if (x === 1) a.push(4); if (seen.length > 10) break; }
print(seen.join(","));
var b = [1, 2, 3, 4];
seen = [];
for (var y of b) { seen.push(y); b.length = 2; }
print(seen.join(","));
