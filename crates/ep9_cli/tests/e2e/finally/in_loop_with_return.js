function find(xs, t) {
  var checks = 0;
  for (var i = 0; i < xs.length; i++) {
    try { if (xs[i] === t) return i + ":" + checks; } finally { checks++; }
  }
  return "none:" + checks;
}
print(find([5, 6, 7], 6), find([5], 9));
