var res = [];
for (var i = 0; i < 2; i++) {
  res.push((function () { inner: for (var j = 0; j < 5; j++) { if (j === 3) break inner; } return j; })());
}
print(res.join(","));
