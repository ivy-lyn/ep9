var o = { a: 1, b: 2, c: 3 };
var seen = [];
outer: for (var k in o) {
  for (var i = 0; i < 2; i++) { if (k === "b") continue outer; seen.push(k + i); }
}
print(seen.join(" "));
