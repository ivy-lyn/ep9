var out = [];
for (var i = 0; i < 10; i++) {
  if (i % 2) continue;
  if (i > 6) break;
  out.push(i);
}
print(out.join(","));
var n = 0;
while (true) { n++; if (n === 7) break; }
print(n);
