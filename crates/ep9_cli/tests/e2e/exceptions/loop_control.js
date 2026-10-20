var out = [];
for (var i = 0; i < 5; i++) {
  try {
    if (i === 1) continue;
    if (i === 3) throw i;
    if (i === 4) break;
    out.push("ok" + i);
  } catch (e) { out.push("caught" + e); }
}
print(out.join(" "));
