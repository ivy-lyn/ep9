var out = [];
outer: for (var i = 0; i < 3; i++) {
  for (var j = 0; j < 3; j++) {
    if (j === 2) continue outer;
    if (i === 2) break outer;
    out.push(i + "" + j);
  }
}
print(out.join(" "));
