var log = [];
a: for (var i = 0; i < 3; i++) {
  b: for (var j = 0; j < 3; j++) {
    c: for (var k = 0; k < 3; k++) {
      if (k === 1) continue b;
      if (j === 1) continue a;
      if (i === 2) break a;
      log.push("" + i + j + k);
    }
  }
}
print(log.join(" "));
